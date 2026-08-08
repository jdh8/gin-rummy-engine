//! Aggregate the fixed EAAI-baseline arena panel from machine-readable legs.
//!
//! `scripts/bench-panel.sh` owns the fixed commands and passes their arena
//! JSON documents here.  This helper validates the protocol, pools the
//! pair-cluster sufficient statistics, and renders Markdown without scraping
//! the arena's human-readable output.

use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

const CANDIDATES: [&str; 3] = ["greedy", "mc:64", "mc:128"];

#[derive(Debug)]
struct Config {
    inputs: Vec<PathBuf>,
    stamp: String,
    round_pairs: u64,
    game_pairs: u64,
    game_pairs_128: u64,
    round_seed: u64,
    seeds: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mode {
    Rounds,
    Games,
}

impl Mode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Rounds => "rounds",
            Self::Games => "games",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LegKey {
    p1: String,
    p2: String,
    mode: Mode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RatioMoments {
    clusters: u64,
    numerator: u64,
    denominator: u64,
    numerator_sq: u64,
    numerator_denominator: u64,
    denominator_sq: u64,
}

impl RatioMoments {
    const fn merge(self, other: Self) -> Self {
        Self {
            clusters: self.clusters + other.clusters,
            numerator: self.numerator + other.numerator,
            denominator: self.denominator + other.denominator,
            numerator_sq: self.numerator_sq + other.numerator_sq,
            numerator_denominator: self.numerator_denominator + other.numerator_denominator,
            denominator_sq: self.denominator_sq + other.denominator_sq,
        }
    }

    fn estimate(self) -> Option<f64> {
        (self.denominator != 0).then(|| self.numerator as f64 / self.denominator as f64)
    }

    /// Pair-cluster normal interval for a ratio of sums.
    fn cluster_interval(self, clamp_rate: bool) -> Option<(f64, f64)> {
        let estimate = self.estimate()?;
        if self.clusters < 2 {
            return None;
        }
        let residual_ss = self.numerator_sq as f64
            - 2.0 * estimate * self.numerator_denominator as f64
            + estimate * estimate * self.denominator_sq as f64;
        let variance = self.clusters as f64 * residual_ss.max(0.0)
            / ((self.clusters - 1) as f64 * (self.denominator as f64).powi(2));
        let half = 1.96 * variance.sqrt();
        let interval = (estimate - half, estimate + half);
        Some(if clamp_rate {
            (interval.0.max(0.0), interval.1.min(1.0))
        } else {
            interval
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PlayerOutcome {
    wins: u64,
    raw_points: u64,
    win_rate: RatioMoments,
    point_rate: RatioMoments,
}

impl PlayerOutcome {
    const fn merge(self, other: Self) -> Self {
        Self {
            wins: self.wins + other.wins,
            raw_points: self.raw_points + other.raw_points,
            win_rate: self.win_rate.merge(other.win_rate),
            point_rate: self.point_rate.merge(other.point_rate),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Outcome {
    trials: u64,
    plays: u64,
    decisive: u64,
    players: [PlayerOutcome; 2],
    sweeps: [u64; 2],
}

impl Outcome {
    const fn merge(self, other: Self) -> Self {
        Self {
            trials: self.trials + other.trials,
            plays: self.plays + other.plays,
            decisive: self.decisive + other.decisive,
            players: [
                self.players[0].merge(other.players[0]),
                self.players[1].merge(other.players[1]),
            ],
            sweeps: [
                self.sweeps[0] + other.sweeps[0],
                self.sweeps[1] + other.sweeps[1],
            ],
        }
    }

    fn exact_p(self) -> f64 {
        exact_sign_p_value(self.sweeps[0], self.sweeps[1]).unwrap_or(1.0)
    }
}

struct BaselineRow {
    bot: &'static str,
    rounds: Outcome,
    games: Outcome,
}

struct Panel {
    rows: Vec<BaselineRow>,
    head_to_head: Outcome,
    reproducibility: Reproducibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Reproducibility {
    source_sha256: String,
    cargo_lock_sha256: String,
    git_head: String,
    git_dirty: bool,
}

fn parse_args() -> Result<Config> {
    let mut inputs = Vec::new();
    let mut stamp = None;
    let mut round_pairs = None;
    let mut game_pairs = None;
    let mut game_pairs_128 = None;
    let mut round_seed = None;
    let mut seeds = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        let mut value = || {
            args.next()
                .with_context(|| format!("{argument} needs a value"))
        };
        match argument.as_str() {
            "--stamp" => stamp = Some(value()?),
            "--round-pairs" => round_pairs = Some(value()?.parse()?),
            "--game-pairs" => game_pairs = Some(value()?.parse()?),
            "--game-pairs-128" => game_pairs_128 = Some(value()?.parse()?),
            "--round-seed" => round_seed = Some(value()?.parse()?),
            "--seeds" => seeds = Some(parse_seeds(&value()?)?),
            flag if flag.starts_with('-') => bail!("unknown flag {flag:?}"),
            path => inputs.push(PathBuf::from(path)),
        }
    }
    ensure!(
        inputs.len() == 7,
        "expected seven arena JSON legs, got {}",
        inputs.len()
    );
    let config = Config {
        inputs,
        stamp: stamp.context("--stamp is required")?,
        round_pairs: round_pairs.context("--round-pairs is required")?,
        game_pairs: game_pairs.context("--game-pairs is required")?,
        game_pairs_128: game_pairs_128.context("--game-pairs-128 is required")?,
        round_seed: round_seed.context("--round-seed is required")?,
        seeds: seeds.context("--seeds is required")?,
    };
    ensure!(config.round_pairs > 0, "--round-pairs must be positive");
    ensure!(config.game_pairs > 0, "--game-pairs must be positive");
    ensure!(
        config.game_pairs_128 > 0,
        "--game-pairs-128 must be positive"
    );
    Ok(config)
}

fn parse_seeds(text: &str) -> Result<Vec<u64>> {
    let mut seeds = Vec::new();
    for field in text.split(|character: char| character == ',' || character.is_whitespace()) {
        if field.is_empty() {
            continue;
        }
        let seed = field
            .parse()
            .with_context(|| format!("invalid seed {field:?}"))?;
        ensure!(!seeds.contains(&seed), "duplicate seed {seed}");
        seeds.push(seed);
    }
    ensure!(!seeds.is_empty(), "--seeds must not be empty");
    Ok(seeds)
}

fn pointer<'a>(document: &'a Value, path: &str) -> Result<&'a Value> {
    document
        .pointer(path)
        .with_context(|| format!("JSON has no {path}"))
}

fn string_at<'a>(document: &'a Value, path: &str) -> Result<&'a str> {
    pointer(document, path)?
        .as_str()
        .with_context(|| format!("{path} is not a string"))
}

fn u64_at(document: &Value, path: &str) -> Result<u64> {
    pointer(document, path)?
        .as_u64()
        .with_context(|| format!("{path} is not an unsigned integer"))
}

fn f64_at(document: &Value, path: &str) -> Result<f64> {
    pointer(document, path)?
        .as_f64()
        .with_context(|| format!("{path} is not numeric"))
}

fn bool_at(document: &Value, path: &str) -> Result<bool> {
    pointer(document, path)?
        .as_bool()
        .with_context(|| format!("{path} is not Boolean"))
}

fn bot_spec<'a>(document: &'a Value, seat: &str) -> Result<&'a str> {
    let bot = pointer(document, &format!("/bots/{seat}"))?;
    bot.as_str()
        .or_else(|| bot.get("spec").and_then(Value::as_str))
        .with_context(|| format!("/bots/{seat} has no string spec"))
}

fn parse_mode(document: &Value) -> Result<Mode> {
    match string_at(document, "/mode")? {
        "rounds" => Ok(Mode::Rounds),
        "games" => Ok(Mode::Games),
        other => bail!("unsupported arena mode {other:?}"),
    }
}

fn validate_rules(document: &Value) -> Result<()> {
    ensure!(string_at(document, "/rules/preset")? == "eaai");
    let expected = json!({
        "knock_limit": 10,
        "oklahoma": null,
        "gin_bonus": 25,
        "big_gin_bonus": null,
        "undercut_bonus": 25,
        "undercut_on_tie": true,
        "box_bonus": 0,
        "immediate_boxes": false,
        "game_bonus": 0,
        "game_target": 100,
        "shutout": {"kind": "flat", "bonus": 0}
    });
    ensure!(
        pointer(document, "/rules/values")? == &expected,
        "arena leg does not use exact EAAI rules"
    );
    Ok(())
}

fn reproducibility(document: &Value) -> Result<Reproducibility> {
    Ok(Reproducibility {
        source_sha256: string_at(document, "/reproducibility/source_sha256")?.to_owned(),
        cargo_lock_sha256: string_at(document, "/reproducibility/cargo_lock_sha256")?.to_owned(),
        git_head: string_at(document, "/reproducibility/git_head")?.to_owned(),
        git_dirty: bool_at(document, "/reproducibility/git_dirty")?,
    })
}

fn moments_at(document: &Value, path: &str) -> Result<RatioMoments> {
    let moments = RatioMoments {
        clusters: u64_at(document, &format!("{path}/clusters"))?,
        numerator: u64_at(document, &format!("{path}/numerator"))?,
        denominator: u64_at(document, &format!("{path}/denominator"))?,
        numerator_sq: u64_at(document, &format!("{path}/numerator_sq"))?,
        numerator_denominator: u64_at(document, &format!("{path}/numerator_denominator"))?,
        denominator_sq: u64_at(document, &format!("{path}/denominator_sq"))?,
    };
    if let Some(estimate) = pointer(document, &format!("{path}/estimate"))?.as_f64() {
        ensure_close(
            estimate,
            moments.estimate().context("moment denominator is zero")?,
            "moment estimate",
        )?;
    }
    Ok(moments)
}

fn ensure_close(left: f64, right: f64, label: &str) -> Result<()> {
    let scale = left.abs().max(right.abs()).max(1.0);
    ensure!(
        (left - right).abs() <= 1e-12 * scale,
        "{label} differs: {left} versus {right}"
    );
    Ok(())
}

fn parse_outcome(document: &Value) -> Result<Outcome> {
    ensure!(u64_at(document, "/failures")? == 0);
    let trials = u64_at(document, "/trials")?;
    let plays = u64_at(document, "/plays")?;
    let decisive = u64_at(document, "/decisive")?;
    let players = pointer(document, "/players")?
        .as_array()
        .context("/players is not an array")?;
    ensure!(players.len() == 2, "an arena outcome must have two players");
    let mut parsed = [PlayerOutcome::default(); 2];
    for (index, player) in players.iter().enumerate() {
        let win_rate = moments_at(player, "/win_rate_moments")?;
        let point_rate = moments_at(player, "/point_rate_moments")?;
        let outcome = PlayerOutcome {
            wins: u64_at(player, "/wins")?,
            raw_points: u64_at(player, "/raw_points")?,
            win_rate,
            point_rate,
        };
        ensure!(outcome.wins == win_rate.numerator);
        ensure!(outcome.raw_points == point_rate.numerator);
        ensure!(win_rate.clusters == trials);
        ensure!(point_rate.clusters == trials);
        ensure!(win_rate.denominator == decisive);
        ensure!(point_rate.denominator == plays);
        parsed[index] = outcome;
    }
    let sweeps = pointer(document, "/comparison/sweeps")?
        .as_array()
        .context("/comparison/sweeps is not an array")?;
    ensure!(sweeps.len() == 2, "comparison must have two sweep counts");
    let sweeps = [
        sweeps[0]
            .as_u64()
            .context("p1 sweep count is not an integer")?,
        sweeps[1]
            .as_u64()
            .context("p2 sweep count is not an integer")?,
    ];
    ensure!(
        string_at(document, "/comparison/primary_test")? == "exact_sweep_sign",
        "paired arena primary test must be exact_sweep_sign"
    );
    let reported_p = f64_at(document, "/comparison/primary_p_value")?;
    ensure!((0.0..=1.0).contains(&reported_p));
    ensure_close(
        reported_p,
        exact_sign_p_value(sweeps[0], sweeps[1]).unwrap_or(1.0),
        "exact pair-sweep p-value",
    )?;
    Ok(Outcome {
        trials,
        plays,
        decisive,
        players: parsed,
        sweeps,
    })
}

fn aggregate_document(document: &Value) -> Result<Outcome> {
    let runs = pointer(document, "/runs")?
        .as_array()
        .context("/runs is not an array")?;
    let aggregate = runs.iter().try_fold(Outcome::default(), |total, run| {
        Ok::<_, anyhow::Error>(total.merge(parse_outcome(pointer(run, "/outcome")?)?))
    })?;
    let pooled = parse_outcome(pointer(document, "/pooled")?)?;
    ensure!(
        aggregate == pooled,
        "pooled outcome does not equal the sum of per-seed sufficient statistics"
    );
    Ok(aggregate)
}

fn validate_player_specs(document: &Value, key: &LegKey) -> Result<()> {
    ensure!(string_at(document, "/players/0/spec")? == key.p1);
    ensure!(string_at(document, "/players/1/spec")? == key.p2);
    Ok(())
}

fn classify(document: &Value) -> Result<LegKey> {
    let p1 = bot_spec(document, "p1")?.to_owned();
    let p2 = bot_spec(document, "p2")?.to_owned();
    let mode = parse_mode(document)?;
    let baseline = CANDIDATES.contains(&p1.as_str()) && p2 == "eaai";
    let head_to_head = p1 == "mc:64" && p2 == "greedy" && mode == Mode::Games;
    ensure!(
        baseline || head_to_head,
        "unexpected baseline-panel leg {p1} vs {p2} ({})",
        mode.as_str()
    );
    ensure!(
        baseline || mode == Mode::Games,
        "head-to-head leg must use games"
    );
    Ok(LegKey { p1, p2, mode })
}

fn validate_document(document: &Value, config: &Config) -> Result<(LegKey, Outcome)> {
    ensure!(string_at(document, "/schema")? == "gin-rummy-arena/v1");
    ensure!(bool_at(document, "/paired")?);
    ensure!(u64_at(document, "/failures")? == 0);
    ensure!(
        string_at(document, "/dealer_rotation")? == "alternate_after_scored_round",
        "every panel leg must use corrected EAAI dealer rotation"
    );
    validate_rules(document)?;

    let key = classify(document)?;
    let expected_pairs = match key.mode {
        Mode::Rounds => config.round_pairs,
        Mode::Games if key.p1 == "mc:128" && key.p2 == "eaai" => config.game_pairs_128,
        Mode::Games => config.game_pairs,
    };
    ensure!(u64_at(document, "/count_per_seed")? == expected_pairs);
    let expected_seeds = match key.mode {
        Mode::Rounds => vec![config.round_seed],
        Mode::Games => config.seeds.clone(),
    };
    let seeds = pointer(document, "/seeds")?
        .as_array()
        .context("/seeds is not an array")?
        .iter()
        .map(|seed| seed.as_u64().context("seed is not an integer"))
        .collect::<Result<Vec<_>>>()?;
    ensure!(seeds == expected_seeds, "arena leg has unexpected seeds");
    let points_metric = match key.mode {
        Mode::Rounds => "round_points",
        Mode::Games => "raw_game_score",
    };
    ensure!(string_at(document, "/points_metric")? == points_metric);

    let runs = pointer(document, "/runs")?
        .as_array()
        .context("/runs is not an array")?;
    ensure!(runs.len() == expected_seeds.len());
    for (run, expected_seed) in runs.iter().zip(&expected_seeds) {
        ensure!(u64_at(run, "/seed")? == *expected_seed);
        ensure!(u64_at(run, "/outcome/trials")? == expected_pairs);
        ensure!(u64_at(run, "/outcome/plays")? == 2 * expected_pairs);
        validate_player_specs(pointer(run, "/outcome")?, &key)?;
    }
    validate_player_specs(pointer(document, "/pooled")?, &key)?;
    let aggregate = aggregate_document(document)?;
    ensure!(aggregate.trials == expected_pairs * expected_seeds.len() as u64);
    ensure!(aggregate.plays == 2 * aggregate.trials);
    match key.mode {
        Mode::Rounds => ensure!(aggregate.decisive <= aggregate.plays),
        Mode::Games => ensure!(aggregate.decisive == aggregate.plays),
    }
    Ok((key, aggregate))
}

fn load_panel(config: &Config) -> Result<Panel> {
    let mut legs = BTreeMap::new();
    let mut shared_reproducibility = None;
    for path in &config.inputs {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let document: Value =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        let (key, outcome) = validate_document(&document, config)
            .with_context(|| format!("validate {}", path.display()))?;
        let identity = reproducibility(&document)
            .with_context(|| format!("read reproducibility from {}", path.display()))?;
        if let Some(expected) = &shared_reproducibility {
            ensure!(
                expected == &identity,
                "arena legs were produced by different source or lockfile states"
            );
        } else {
            shared_reproducibility = Some(identity);
        }
        ensure!(legs.insert(key, outcome).is_none(), "duplicate arena leg");
    }

    let mut rows = Vec::with_capacity(CANDIDATES.len());
    for bot in CANDIDATES {
        let rounds = take_leg(&mut legs, bot, "eaai", Mode::Rounds)?;
        let games = take_leg(&mut legs, bot, "eaai", Mode::Games)?;
        rows.push(BaselineRow { bot, rounds, games });
    }
    let head_to_head = take_leg(&mut legs, "mc:64", "greedy", Mode::Games)?;
    ensure!(legs.is_empty(), "unexpected extra arena legs");
    Ok(Panel {
        rows,
        head_to_head,
        reproducibility: shared_reproducibility.context("no arena reproducibility metadata")?,
    })
}

fn take_leg(
    legs: &mut BTreeMap<LegKey, Outcome>,
    p1: &str,
    p2: &str,
    mode: Mode,
) -> Result<Outcome> {
    legs.remove(&LegKey {
        p1: p1.to_owned(),
        p2: p2.to_owned(),
        mode,
    })
    .with_context(|| format!("missing {} leg {p1} vs {p2}", mode.as_str()))
}

fn exact_sign_p_value(positive: u64, negative: u64) -> Option<f64> {
    let n = positive + negative;
    if n == 0 {
        return None;
    }
    let tail = positive.min(negative);
    let log_combination = (1..=tail).fold(0.0, |sum, index| {
        sum + ((n - tail + index) as f64).ln() - (index as f64).ln()
    });
    let log_largest = log_combination - n as f64 * std::f64::consts::LN_2;
    let mut relative_term = 1.0;
    let mut relative_sum = 1.0;
    for index in (1..=tail).rev() {
        relative_term *= index as f64 / (n - index + 1) as f64;
        relative_sum += relative_term;
    }
    Some((2.0 * log_largest.exp() * relative_sum).min(1.0))
}

fn percent(value: f64) -> String {
    format!("{:.1}%", 100.0 * value)
}

fn rate_with_interval(moments: RatioMoments) -> Result<String> {
    let Some(estimate) = moments.estimate() else {
        return Ok("n/a (no decisive hands)".to_owned());
    };
    Ok(match moments.cluster_interval(true) {
        Some((low, high)) => format!("{} ({}–{})", percent(estimate), percent(low), percent(high)),
        None => format!("{} (CI n/a)", percent(estimate)),
    })
}

fn mean(moments: RatioMoments) -> Result<f64> {
    moments.estimate().context("mean denominator is zero")
}

fn p_value(value: f64) -> String {
    if value < 0.001 {
        "<0.001".to_owned()
    } else {
        format!("{value:.3}")
    }
}

fn render(config: &Config, panel: &Panel) -> Result<String> {
    let seeds = config
        .seeds
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let mut output = String::new();
    writeln!(
        output,
        "<!-- scripts/bench-panel.sh at {}: ROUND_PAIRS={} GAME_PAIRS={} GAME_PAIRS_128={} ROUND_SEED={} SEEDS=\"{}\" -->\n",
        config.stamp,
        config.round_pairs,
        config.game_pairs,
        config.game_pairs_128,
        config.round_seed,
        seeds,
    )?;
    writeln!(
        output,
        "<!-- arena source SHA-256: {}; Cargo.lock SHA-256: {}; git: {}{} -->\n",
        panel.reproducibility.source_sha256,
        panel.reproducibility.cargo_lock_sha256,
        panel.reproducibility.git_head,
        if panel.reproducibility.git_dirty {
            " (dirty)"
        } else {
            ""
        },
    )?;
    writeln!(
        output,
        "Pair-cluster 95% intervals are computed from arena sufficient moments. Exact p-values are two-sided sign tests over 2–0 mirrored-pair sweeps; split pairs are ties. Game scores are raw target-reaching totals.\n"
    )?;
    writeln!(
        output,
        "| Bot vs baseline | Decisive rounds won | Points/round | Games won | Raw score/game | Game sweeps | Exact p |"
    )?;
    writeln!(output, "|---|---:|---:|---:|---:|---:|---:|")?;
    for row in &panel.rows {
        writeln!(
            output,
            "| `{}` | {} | {:.2} vs {:.2} | {} | {:.2} vs {:.2} | {}–{} | {} |",
            row.bot,
            rate_with_interval(row.rounds.players[0].win_rate)?,
            mean(row.rounds.players[0].point_rate)?,
            mean(row.rounds.players[1].point_rate)?,
            rate_with_interval(row.games.players[0].win_rate)?,
            mean(row.games.players[0].point_rate)?,
            mean(row.games.players[1].point_rate)?,
            row.games.sweeps[0],
            row.games.sweeps[1],
            p_value(row.games.exact_p()),
        )?;
    }

    let head = panel.head_to_head;
    writeln!(
        output,
        "\nhead-to-head, `mc:64` vs `greedy`: {} of {} games; raw score/game {:.2} vs {:.2}; sweeps {}–{}; exact pair-sweep p = {}.",
        rate_with_interval(head.players[0].win_rate)?,
        head.plays,
        mean(head.players[0].point_rate)?,
        mean(head.players[1].point_rate)?,
        head.sweeps[0],
        head.sweeps[1],
        p_value(head.exact_p()),
    )?;
    Ok(output)
}

fn main() -> Result<()> {
    let config = parse_args()?;
    let panel = load_panel(&config)?;
    print!("{}", render(&config, &panel)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moments_value(moments: RatioMoments) -> Value {
        json!({
            "clusters": moments.clusters,
            "numerator": moments.numerator,
            "denominator": moments.denominator,
            "numerator_sq": moments.numerator_sq,
            "numerator_denominator": moments.numerator_denominator,
            "denominator_sq": moments.denominator_sq,
            "estimate": moments.estimate(),
            "cluster_ci95": null
        })
    }

    fn player_value(player: PlayerOutcome, spec: &str) -> Value {
        json!({
            "spec": spec,
            "wins": player.wins,
            "raw_points": player.raw_points,
            "win_rate_moments": moments_value(player.win_rate),
            "point_rate_moments": moments_value(player.point_rate)
        })
    }

    fn outcome_value(outcome: Outcome) -> Value {
        json!({
            "trials": outcome.trials,
            "plays": outcome.plays,
            "decisive": outcome.decisive,
            "failures": 0,
            "players": [
                player_value(outcome.players[0], "greedy"),
                player_value(outcome.players[1], "eaai")
            ],
            "comparison": {
                "sweeps": outcome.sweeps,
                "primary_test": "exact_sweep_sign",
                "primary_p_value": outcome.exact_p()
            }
        })
    }

    fn seed_outcomes() -> [Outcome; 2] {
        [
            Outcome {
                trials: 2,
                plays: 4,
                decisive: 4,
                players: [
                    PlayerOutcome {
                        wins: 3,
                        raw_points: 60,
                        win_rate: RatioMoments {
                            clusters: 2,
                            numerator: 3,
                            denominator: 4,
                            numerator_sq: 5,
                            numerator_denominator: 6,
                            denominator_sq: 8,
                        },
                        point_rate: RatioMoments {
                            clusters: 2,
                            numerator: 60,
                            denominator: 4,
                            numerator_sq: 2_000,
                            numerator_denominator: 120,
                            denominator_sq: 8,
                        },
                    },
                    PlayerOutcome {
                        wins: 1,
                        raw_points: 50,
                        win_rate: RatioMoments {
                            clusters: 2,
                            numerator: 1,
                            denominator: 4,
                            numerator_sq: 1,
                            numerator_denominator: 2,
                            denominator_sq: 8,
                        },
                        point_rate: RatioMoments {
                            clusters: 2,
                            numerator: 50,
                            denominator: 4,
                            numerator_sq: 1_300,
                            numerator_denominator: 100,
                            denominator_sq: 8,
                        },
                    },
                ],
                sweeps: [1, 0],
            },
            Outcome {
                trials: 2,
                plays: 4,
                decisive: 4,
                players: [
                    PlayerOutcome {
                        wins: 1,
                        raw_points: 40,
                        win_rate: RatioMoments {
                            clusters: 2,
                            numerator: 1,
                            denominator: 4,
                            numerator_sq: 1,
                            numerator_denominator: 2,
                            denominator_sq: 8,
                        },
                        point_rate: RatioMoments {
                            clusters: 2,
                            numerator: 40,
                            denominator: 4,
                            numerator_sq: 1_000,
                            numerator_denominator: 80,
                            denominator_sq: 8,
                        },
                    },
                    PlayerOutcome {
                        wins: 3,
                        raw_points: 70,
                        win_rate: RatioMoments {
                            clusters: 2,
                            numerator: 3,
                            denominator: 4,
                            numerator_sq: 5,
                            numerator_denominator: 6,
                            denominator_sq: 8,
                        },
                        point_rate: RatioMoments {
                            clusters: 2,
                            numerator: 70,
                            denominator: 4,
                            numerator_sq: 2_500,
                            numerator_denominator: 140,
                            denominator_sq: 8,
                        },
                    },
                ],
                sweeps: [0, 1],
            },
        ]
    }

    fn document_with_runs(outcomes: [Outcome; 2]) -> Value {
        let pooled = outcomes[0].merge(outcomes[1]);
        json!({
            "runs": [
                {"seed": 7, "outcome": outcome_value(outcomes[0])},
                {"seed": 8, "outcome": outcome_value(outcomes[1])}
            ],
            "pooled": outcome_value(pooled)
        })
    }

    fn valid_round_document() -> Value {
        let outcome = seed_outcomes()[0];
        json!({
            "schema": "gin-rummy-arena/v1",
            "mode": "rounds",
            "paired": true,
            "count_per_seed": 2,
            "failures": 0,
            "rules": {
                "preset": "eaai",
                "values": {
                    "knock_limit": 10,
                    "oklahoma": null,
                    "gin_bonus": 25,
                    "big_gin_bonus": null,
                    "undercut_bonus": 25,
                    "undercut_on_tie": true,
                    "box_bonus": 0,
                    "immediate_boxes": false,
                    "game_bonus": 0,
                    "game_target": 100,
                    "shutout": {"kind": "flat", "bonus": 0}
                }
            },
            "dealer_rotation": "alternate_after_scored_round",
            "points_metric": "round_points",
            "bots": {
                "p1": {"spec": "greedy", "configuration": {}},
                "p2": {"spec": "eaai", "configuration": {}}
            },
            "seeds": [7],
            "runs": [{"seed": 7, "outcome": outcome_value(outcome)}],
            "pooled": outcome_value(outcome)
        })
    }

    fn test_config() -> Config {
        Config {
            inputs: Vec::new(),
            stamp: "abc123-dirty".to_owned(),
            round_pairs: 2,
            game_pairs: 2,
            game_pairs_128: 1,
            round_seed: 7,
            seeds: vec![7, 8],
        }
    }

    #[test]
    fn aggregation_uses_per_seed_sufficient_moments() {
        let aggregate = aggregate_document(&document_with_runs(seed_outcomes()))
            .expect("valid per-seed arena outcomes aggregate");
        assert_eq!(aggregate.trials, 4);
        assert_eq!(aggregate.players[0].win_rate.numerator, 4);
        assert_eq!(aggregate.players[0].win_rate.denominator, 8);
        assert_eq!(aggregate.players[0].point_rate.estimate(), Some(12.5));
        assert_eq!(aggregate.sweeps, [1, 1]);
        assert_eq!(aggregate.exact_p(), 1.0);
        let interval = aggregate.players[0]
            .win_rate
            .cluster_interval(true)
            .expect("four pairs have a cluster interval");
        assert!(interval.0 < 0.5 && interval.1 > 0.5);
    }

    #[test]
    fn aggregation_rejects_a_pooled_textbook_count_without_matching_moments() {
        let mut document = document_with_runs(seed_outcomes());
        *document
            .pointer_mut("/pooled/players/0/win_rate_moments/numerator_sq")
            .expect("fixture has pooled moments") = json!(99);
        let error = aggregate_document(&document).expect_err("bad pooled moments must fail");
        assert!(error.to_string().contains("does not equal"));
    }

    #[test]
    fn validation_requires_corrected_eaai_protocol() {
        let config = test_config();
        validate_document(&valid_round_document(), &config)
            .expect("the corrected EAAI fixture validates");

        let mut stale_rotation = valid_round_document();
        *stale_rotation
            .pointer_mut("/dealer_rotation")
            .expect("fixture has a dealer rotation") = json!("winner_deals");
        let error = validate_document(&stale_rotation, &config)
            .expect_err("winner-deals must not enter the EAAI panel");
        assert!(error.to_string().contains("corrected EAAI dealer"));

        let mut stale_bonus = valid_round_document();
        *stale_bonus
            .pointer_mut("/rules/values/game_bonus")
            .expect("fixture has a game bonus") = json!(100);
        let error = validate_document(&stale_bonus, &config)
            .expect_err("modern game bonuses must not enter the EAAI panel");
        assert!(error.to_string().contains("exact EAAI rules"));
    }

    #[test]
    fn renderer_includes_cluster_inference_raw_scores_and_exact_sweeps() {
        let aggregate = seed_outcomes()[0].merge(seed_outcomes()[1]);
        let panel = Panel {
            rows: CANDIDATES
                .into_iter()
                .map(|bot| BaselineRow {
                    bot,
                    rounds: aggregate,
                    games: aggregate,
                })
                .collect(),
            head_to_head: aggregate,
            reproducibility: Reproducibility {
                source_sha256: "source".to_owned(),
                cargo_lock_sha256: "lock".to_owned(),
                git_head: "abcdef".to_owned(),
                git_dirty: true,
            },
        };
        let rendered = render(&test_config(), &panel).expect("fixture renders");
        assert!(rendered.contains("Pair-cluster 95% intervals"));
        assert!(rendered.contains("| `greedy` | 50.0% ("));
        assert!(rendered.contains("| 12.50 vs 15.00 | 1–1 | 1.000 |"));
        assert!(rendered.contains("raw score/game 12.50 vs 15.00"));
        assert!(rendered.contains("exact pair-sweep p = 1.000"));
        assert!(rendered.contains("SEEDS=\"7 8\""));
        assert!(rendered.contains("arena source SHA-256: source"));
    }

    #[test]
    fn seed_parser_accepts_the_legacy_space_separated_override() {
        assert_eq!(parse_seeds("7 8").expect("legacy seed list"), [7, 8]);
        assert_eq!(parse_seeds("7,8").expect("arena seed list"), [7, 8]);
        assert!(parse_seeds("7 7").is_err());
    }
}
