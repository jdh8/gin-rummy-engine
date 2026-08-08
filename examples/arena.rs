//! Bot-vs-bot tournaments with win-rate statistics.
//!
//! ```console
//! cargo run --release --example arena -- --rounds 1000 --p1 greedy --p2 mc:32
//! cargo run --release --example arena -- --games 3000 --p1 mc:64 --p2 eaai \
//!   --rules eaai --alternate-dealer --seeds 7,8 --format json
//! ```
//!
//! Trials are seeded by index and fan out across the CPUs, so a given
//! `--seed` reproduces the same counts however the work schedules.  By
//! default every trial is a *mirrored pair*: both bots swap seats under
//! common-random-number deal streams, and the printed significance line
//! tests the paired per-trial difference.  `--unpaired` plays a single game per trial
//! (seats alternating by index) instead.  `--rounds N` plays N trials of
//! independent single rounds; `--games N` plays N trials of whole games
//! to the target score, where `--alternate-dealer` flips the deal after
//! every scored round as the EAAI challenge did, rather than gin rummy's
//! usual winner-deals-next.  A dead hand is redealt by the same dealer.
//!
//! Build without `--features parallel`: Monte Carlo's in-decision
//! parallelism would compete with the trial-level fan-out for the same
//! rayon pool.

mod support;

use anyhow::{Context as _, Result, bail};
use gin_rummy::{FinalScore, Game, OklahomaAce, Player, Round, RoundResult, Rules, Shutout};
use gin_rummy_engine::{
    DealerRotation, EaaiSimpleBot, GameValue, HeuristicBot, McConfig, MonteCarloBot, Strategy,
    Table, eaai_rules,
};
use rand::rngs::StdRng;
use rand::{RngExt as _, SeedableRng};
use rayon::prelude::*;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;
use support::arena_stats::{
    Interval, RatioMoments, SignedRatioMoments, exact_sign_p_value, normal_p_value, wilson,
};

/// SplitMix64's increment: mixed into the seed so adjacent trial indices
/// get decorrelated random streams.
const MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// Offsets putting each bot's generator on its own stream, independent of
/// the deal stream.  Within a mirrored pair each bot is re-seeded
/// identically in both orientations, so the deals are the only difference
/// between the two runs.
const BOT_STREAM: [u64; 2] = [0xD1B5_4A32_D192_ED03, 0x94D0_49BB_1331_11EB];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

struct Config {
    count: u32,
    games: bool,
    paired: bool,
    alternate_dealer: bool,
    p1: String,
    p2: String,
    seeds: Vec<u64>,
    rules: Rules,
    rules_name: &'static str,
    format: OutputFormat,
}

fn parse_args() -> Result<Config> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from(args: impl IntoIterator<Item = String>) -> Result<Config> {
    let mut config = Config {
        count: 200,
        games: false,
        paired: true,
        alternate_dealer: false,
        p1: "greedy".into(),
        p2: "mc".into(),
        seeds: Vec::new(),
        rules: Rules::default(),
        rules_name: "modern",
        format: OutputFormat::Text,
    };
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = || args.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--rounds" => {
                config.count = value()?.parse()?;
                config.games = false;
            }
            "--games" => {
                config.count = value()?.parse()?;
                config.games = true;
            }
            "--paired" => config.paired = true,
            "--unpaired" => config.paired = false,
            "--alternate-dealer" => config.alternate_dealer = true,
            "--p1" => config.p1 = value()?,
            "--p2" => config.p2 = value()?,
            "--seed" => push_seed(&mut config.seeds, value()?.parse()?)?,
            "--seeds" => {
                let seeds = value()?;
                if seeds.is_empty() {
                    bail!("--seeds needs a comma-separated non-empty list");
                }
                for seed in seeds.split(',') {
                    if seed.is_empty() {
                        bail!("--seeds contains an empty seed");
                    }
                    push_seed(&mut config.seeds, seed.parse()?)?;
                }
            }
            "--format" => {
                config.format = match value()?.as_str() {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    other => bail!("unknown output format {other:?} (text | json)"),
                }
            }
            "--rules" => {
                let preset = value()?;
                let (rules, name) = match preset.as_str() {
                    "modern" => (Rules::new(), "modern"),
                    "classic" => (Rules::classic(), "classic"),
                    "palace" => (Rules::palace(), "palace"),
                    // The exact EAAI-2021 scoring preset, without deferred bonuses.
                    "eaai" => (eaai_rules(), "eaai"),
                    other => bail!("unknown rules preset {other:?}"),
                };
                config.rules = rules;
                config.rules_name = name;
            }
            other => bail!(
                "unknown flag {other:?} (--rounds/--games/--paired/--unpaired/\
                 --alternate-dealer/--p1/--p2/--seed/--seeds/--rules/--format)"
            ),
        }
    }
    if config.count == 0 {
        bail!("the trial count must be greater than zero");
    }
    Ok(config)
}

fn push_seed(seeds: &mut Vec<u64>, seed: u64) -> Result<()> {
    if seeds.contains(&seed) {
        bail!("duplicate seed {seed}");
    }
    seeds.push(seed);
    Ok(())
}

fn make_bot(spec: &str, seed: u64) -> Result<Box<dyn Strategy>> {
    if let Some(bot) = support::strong::make_bot(spec, seed)? {
        return Ok(bot);
    }
    let (kind, samples) = match spec.split_once(':') {
        Some((kind, samples)) => (kind, Some(samples.parse::<u32>()?)),
        None => (spec, None),
    };
    match kind {
        "greedy" => Ok(Box::new(HeuristicBot::new())),
        // The EAAI-2021 challenge baseline, the cross-engine yardstick.
        "eaai" => Ok(Box::new(EaaiSimpleBot::new(StdRng::seed_from_u64(seed)))),
        "mc" => Ok(Box::new(
            MonteCarloBot::new(StdRng::seed_from_u64(seed)).samples(samples.unwrap_or(128)),
        )),
        // Monte Carlo with the historical affine equity in place of the
        // default game-win value function.  This is the arm that default was
        // measured against, kept so the comparison stays reproducible.
        "mca" => {
            let mut config = McConfig::new();
            config.samples = samples.unwrap_or(128);
            config.game_value = GameValue::Affine;
            Ok(Box::new(MonteCarloBot::with_config(
                StdRng::seed_from_u64(seed),
                config,
            )))
        }
        other => {
            let strong = support::strong::BOT_SPECS.join(" | ");
            bail!("unknown bot {other:?} (greedy | eaai | mc[:samples] | mca[:samples] | {strong})")
        }
    }
}

/// Everything a trial reports, summable across trials in any order
#[derive(Clone, Default)]
struct Outcome {
    /// Decisive round wins (rounds mode) or game wins per bot
    wins: [u32; 2],
    /// Round points won, or raw running-score totals across finished games.
    points: [u64; 2],
    knocks: [u32; 2],
    undercuts: [u32; 2],
    gins: [u32; 2],
    big_gins: [u32; 2],
    dead: u32,
    /// Trials merged in — the paired test's sample size
    trials: u32,
    /// Sum of per-trial win differences, bot 1 minus bot 2, and of their
    /// squares: the sufficient statistics of the paired significance test
    d_sum: i64,
    d2_sum: u64,
    /// Pair-cluster moments for each bot's decisive-win rate.
    rates: [RatioMoments; 2],
    /// Pair-cluster moments for points per round or raw score per game.
    point_rates: [RatioMoments; 2],
    /// Pair-cluster moments for p1 minus p2 points per play.
    margin_rate: SignedRatioMoments,
    /// Pairs swept by each bot: it won both mirrored orientations.
    sweeps: [u32; 2],
}

impl Outcome {
    fn merge(mut self, other: Self) -> Self {
        for bot in 0..2 {
            self.wins[bot] += other.wins[bot];
            self.points[bot] += other.points[bot];
            self.knocks[bot] += other.knocks[bot];
            self.undercuts[bot] += other.undercuts[bot];
            self.gins[bot] += other.gins[bot];
            self.big_gins[bot] += other.big_gins[bot];
        }
        self.dead += other.dead;
        self.trials += other.trials;
        self.d_sum += other.d_sum;
        self.d2_sum += other.d2_sum;
        for bot in 0..2 {
            self.rates[bot] = self.rates[bot].merge(other.rates[bot]);
            self.point_rates[bot] = self.point_rates[bot].merge(other.point_rates[bot]);
            self.sweeps[bot] += other.sweeps[bot];
        }
        self.margin_rate = self.margin_rate.merge(other.margin_rate);
        self
    }

    /// Record a round result; `bot_of_seat` maps each seat to a bot index.
    fn record_round(&mut self, result: RoundResult, rules: &Rules, bot_of_seat: [usize; 2]) {
        self.record_finish(result, bot_of_seat);
        if let Some(winner) = result.winner() {
            let bot = bot_of_seat[winner as usize];
            self.wins[bot] += 1;
            self.points[bot] += u64::from(result.points(rules));
        }
    }

    fn record_finish(&mut self, result: RoundResult, bot_of_seat: [usize; 2]) {
        match result {
            RoundResult::Dead => self.dead += 1,
            RoundResult::Knock { winner, .. } => self.knocks[bot_of_seat[winner as usize]] += 1,
            RoundResult::Undercut { winner, .. } => {
                self.undercuts[bot_of_seat[winner as usize]] += 1;
            }
            RoundResult::Gin { winner, .. } => self.gins[bot_of_seat[winner as usize]] += 1,
            RoundResult::BigGin { winner, .. } => {
                self.big_gins[bot_of_seat[winner as usize]] += 1;
            }
            _ => {}
        }
    }

    fn merge_finishes(&mut self, other: &Self) {
        self.dead += other.dead;
        for bot in 0..2 {
            self.knocks[bot] += other.knocks[bot];
            self.undercuts[bot] += other.undercuts[bot];
            self.gins[bot] += other.gins[bot];
            self.big_gins[bot] += other.big_gins[bot];
        }
    }
}

struct PlayedGame {
    score: FinalScore,
    raw_scores: [u16; 2],
    finishes: Outcome,
}

fn winner_deals_game(
    rules: Rules,
    first_dealer: Player,
    strategies: [&mut dyn Strategy; 2],
    bot_of_seat: [usize; 2],
    rng: &mut StdRng,
) -> Result<PlayedGame> {
    let [one, two] = strategies;
    let mut game = Game::new(rules, first_dealer);
    let mut finishes = Outcome::default();
    while !game.is_over() {
        let scores = [game.score(Player::One), game.score(Player::Two)];
        let mut table = Table::new(game.deal(rng)).scores(scores);
        let result = table.play([&mut *one, &mut *two])?;
        finishes.record_finish(result, bot_of_seat);
        game.record(result)?;
    }
    let raw_scores = [game.score(Player::One), game.score(Player::Two)];
    Ok(PlayedGame {
        score: game.final_score().expect("a game that is over settles"),
        raw_scores,
        finishes,
    })
}

/// Play a whole game alternating the dealer after each scored round — the
/// EAAI challenge's protocol — instead of [`Game`]'s winner-deals-next.
/// A dead hand is redealt by the same dealer.  The `Game` serves purely as
/// the scoreboard.
fn alternate_after_scored_round_game(
    rules: Rules,
    first_dealer: Player,
    strategies: [&mut dyn Strategy; 2],
    bot_of_seat: [usize; 2],
    rng: &mut StdRng,
) -> Result<PlayedGame> {
    let [one, two] = strategies;
    let mut game = Game::new(rules, first_dealer);
    let mut dealer = first_dealer;
    let mut finishes = Outcome::default();
    while !game.is_over() {
        let scores = [game.score(Player::One), game.score(Player::Two)];
        let mut table = Table::new(Round::deal(rules, dealer, rng))
            .scores(scores)
            .dealer_rotation(DealerRotation::AlternateAfterScoredRound);
        let result = table.play([&mut *one, &mut *two])?;
        finishes.record_finish(result, bot_of_seat);
        game.record(result)?;
        dealer = challenge_next_dealer(dealer, result);
    }
    let raw_scores = [game.score(Player::One), game.score(Player::Two)];
    Ok(PlayedGame {
        score: game.final_score().expect("a game that is over settles"),
        raw_scores,
        finishes,
    })
}

fn challenge_next_dealer(dealer: Player, result: RoundResult) -> Player {
    if result.winner().is_some() {
        dealer.opponent()
    } else {
        dealer
    }
}

/// Play trial `index`: one round or game per orientation, mirrored across
/// seats in paired mode
///
/// The deal stream and both bots are seeded from the index alone, so the
/// two orientations of a pair differ *only* in seating, and re-running a
/// seed reproduces every trial regardless of scheduling.
fn trial(config: &Config, seed: u64, index: u32) -> Result<Outcome> {
    let trial_seed = seed ^ u64::from(index).wrapping_mul(MIX);
    let dealer = if index % 4 < 2 {
        Player::One
    } else {
        Player::Two
    };
    let orientations: &[bool] = if config.paired {
        &[false, true]
    } else {
        &[index % 2 == 1]
    };

    // In rounds mode both orientations replay this exact deal.
    let dealt = (!config.games).then(|| {
        let table = Table::deal(config.rules, dealer, &mut StdRng::seed_from_u64(trial_seed));
        if config.alternate_dealer {
            table.dealer_rotation(DealerRotation::AlternateAfterScoredRound)
        } else {
            table
        }
    });

    let mut outcome = Outcome {
        trials: 1,
        ..Outcome::default()
    };
    let mut d = 0i64;
    for &swapped in orientations {
        let mut one = make_bot(&config.p1, trial_seed ^ BOT_STREAM[0])?;
        let mut two = make_bot(&config.p2, trial_seed ^ BOT_STREAM[1])?;
        let bot_of_seat = if swapped { [1, 0] } else { [0, 1] };
        let seats: [&mut dyn Strategy; 2] = if swapped {
            [&mut *two, &mut *one]
        } else {
            [&mut *one, &mut *two]
        };

        if config.games {
            // Identically seeded deal streams pair the orientations.  A
            // dead hand can make the dealer sequences diverge, but each
            // orientation still consumes the same indexed shuffle stream.
            let mut rng = StdRng::seed_from_u64(trial_seed);
            let played = if config.alternate_dealer {
                alternate_after_scored_round_game(
                    config.rules,
                    dealer,
                    seats,
                    bot_of_seat,
                    &mut rng,
                )?
            } else {
                winner_deals_game(config.rules, dealer, seats, bot_of_seat, &mut rng)?
            };
            let bot = bot_of_seat[played.score.winner as usize];
            outcome.wins[bot] += 1;
            for seat in Player::ALL {
                outcome.points[bot_of_seat[seat as usize]] +=
                    u64::from(played.raw_scores[seat as usize]);
            }
            outcome.merge_finishes(&played.finishes);
            d += if bot == 0 { 1 } else { -1 };
        } else {
            let mut table = dealt.as_ref().expect("rounds mode deals upfront").clone();
            let result = table.play(seats)?;
            outcome.record_round(result, &config.rules, bot_of_seat);
            if let Some(winner) = result.winner() {
                d += if bot_of_seat[winner as usize] == 0 {
                    1
                } else {
                    -1
                };
            }
        }
    }
    outcome.d_sum = d;
    outcome.d2_sum = (d * d) as u64;
    let decisive = outcome.wins[0] + outcome.wins[1];
    let plays = u32::try_from(orientations.len()).expect("at most two orientations");
    for bot in 0..2 {
        outcome.rates[bot].record(outcome.wins[bot], decisive);
        outcome.point_rates[bot].record(
            u32::try_from(outcome.points[bot]).expect("one trial's points fit in u32"),
            plays,
        );
    }
    let margin = i64::try_from(outcome.points[0]).expect("one trial's points fit in i64")
        - i64::try_from(outcome.points[1]).expect("one trial's points fit in i64");
    outcome.margin_rate.record(
        i32::try_from(margin).expect("one trial's score margin fits in i32"),
        plays,
    );
    if config.paired && outcome.wins[0] == 2 {
        outcome.sweeps[0] = 1;
    } else if config.paired && outcome.wins[1] == 2 {
        outcome.sweeps[1] = 1;
    }
    Ok(outcome)
}

#[derive(Debug, Clone, Copy)]
struct Comparison {
    difference: f64,
    normal_z: Option<f64>,
    infinite_z_direction: i8,
    normal_p: Option<f64>,
    exact_p: Option<f64>,
}

fn comparison(outcome: &Outcome, config: &Config, denominator: u32) -> Comparison {
    let difference =
        (f64::from(outcome.wins[0]) - f64::from(outcome.wins[1])) / f64::from(denominator.max(1));
    if config.paired {
        let n = f64::from(outcome.trials);
        let mean = outcome.d_sum as f64 / n;
        let variance = if outcome.trials > 1 {
            ((outcome.d2_sum as f64) - n * mean * mean) / f64::from(outcome.trials - 1)
        } else {
            0.0
        };
        let (normal_z, infinite_z_direction, normal_p) = if variance > 0.0 {
            let z = mean / (variance / n).sqrt();
            (Some(z), 0, Some(normal_p_value(z)))
        } else if mean > 0.0 {
            (None, 1, Some(0.0))
        } else if mean < 0.0 {
            (None, -1, Some(0.0))
        } else {
            (None, 0, None)
        };
        Comparison {
            difference,
            normal_z,
            infinite_z_direction,
            normal_p,
            // With no sweeps the exact test has no evidence against its
            // null; report p=1 so a fixed multi-matchup panel remains
            // directly consumable by Holm correction.
            exact_p: Some(exact_sign_p_value(outcome.sweeps[0], outcome.sweeps[1]).unwrap_or(1.0)),
        }
    } else {
        let decisive = outcome.wins[0] + outcome.wins[1];
        let normal_z = (decisive != 0).then(|| {
            (f64::from(outcome.wins[0]) - f64::from(outcome.wins[1])) / f64::from(decisive).sqrt()
        });
        Comparison {
            difference,
            normal_z,
            infinite_z_direction: 0,
            normal_p: normal_z.map(normal_p_value),
            exact_p: Some(exact_sign_p_value(outcome.wins[0], outcome.wins[1]).unwrap_or(1.0)),
        }
    }
}

fn rate_interval(
    outcome: &Outcome,
    config: &Config,
    bot: usize,
    denominator: u32,
) -> (&'static str, Interval) {
    if config.paired
        && let Some(interval) = outcome.rates[bot].cluster_interval()
    {
        ("pair_cluster_normal", interval)
    } else {
        ("wilson", wilson(outcome.wins[bot], denominator))
    }
}

#[derive(Clone)]
struct SeedRun {
    seed: u64,
    outcome: Outcome,
    elapsed: std::time::Duration,
}

fn run_seed(config: &Config, seed: u64) -> Result<SeedRun> {
    let start = Instant::now();
    let outcome = (0..config.count)
        .into_par_iter()
        .map(|index| trial(config, seed, index))
        .try_reduce(Outcome::default, |a, b| Ok(a.merge(b)))?;
    Ok(SeedRun {
        seed,
        outcome,
        elapsed: start.elapsed(),
    })
}

fn total_plays(outcome: &Outcome, config: &Config) -> u32 {
    outcome.trials * if config.paired { 2 } else { 1 }
}

fn print_significance(outcome: &Outcome, config: &Config, denominator: u32) {
    let comparison = comparison(outcome, config, denominator);
    print!(
        "p1={} vs p2={}: diff {:+.1}pp",
        config.p1,
        config.p2,
        100.0 * comparison.difference,
    );
    if config.paired {
        if let Some(z) = comparison.normal_z {
            print!(
                ", paired z = {z:.2}, p = {:.3}",
                comparison.normal_p.expect("a finite z has a p-value"),
            );
        } else if comparison.infinite_z_direction != 0 {
            let sign = if comparison.infinite_z_direction > 0 {
                "+"
            } else {
                "-"
            };
            print!(", paired z = {sign}inf, p = 0.000");
        } else {
            print!(", no nonzero paired differences");
        }
        if outcome.sweeps[0] + outcome.sweeps[1] == 0 {
            print!(", no sweeps, exact sign p = 1.000");
        } else {
            print!(
                ", sweeps {}-{}, exact sign p = {p:.3}",
                outcome.sweeps[0],
                outcome.sweeps[1],
                p = comparison.exact_p.expect("the exact p-value is total"),
            );
        }
    } else if let Some(z) = comparison.normal_z {
        print!(
            ", sign z = {z:.2}, normal p = {:.3}, exact p = {:.3}",
            comparison.normal_p.expect("a finite z has a p-value"),
            comparison
                .exact_p
                .expect("a decisive result has an exact p-value"),
        );
    } else {
        print!(", no decisive trials");
    }
    println!();
}

fn print_text_outcome(
    outcome: &Outcome,
    config: &Config,
    elapsed: std::time::Duration,
    seed_label: &str,
) {
    let total = total_plays(outcome, config);
    let unit = if config.games { "game" } else { "round" };
    let rate = f64::from(total) / elapsed.as_secs_f64();
    if config.paired {
        println!(
            "{} mirrored pairs ({total} {unit}s) in {elapsed:.2?} ({rate:.1} {unit}s/s), {seed_label}",
            outcome.trials,
        );
    } else {
        println!("{total} {unit}s in {elapsed:.2?} ({rate:.1} {unit}s/s), {seed_label}");
    }

    let names = [format!("p1={}", config.p1), format!("p2={}", config.p2)];
    let decisive = outcome.wins[0] + outcome.wins[1];
    let denominator = if config.games { total } else { decisive };
    for (bot, name) in names.iter().enumerate() {
        let (method, interval) = rate_interval(outcome, config, bot, denominator);
        let label = if method == "pair_cluster_normal" {
            "95% pair-cluster CI"
        } else {
            "95% CI"
        };
        if config.games {
            println!(
                "{name}: {} game wins / {total} ({:.1}%, {label} {:.1}%\u{2013}{:.1}%), {} raw points",
                outcome.wins[bot],
                100.0 * f64::from(outcome.wins[bot]) / f64::from(total.max(1)),
                100.0 * interval.low,
                100.0 * interval.high,
                outcome.points[bot],
            );
        } else {
            println!(
                "{name}: {} wins / {decisive} decisive ({:.1}%, {label} {:.1}%\u{2013}{:.1}%), {:.2} points/round",
                outcome.wins[bot],
                100.0 * f64::from(outcome.wins[bot]) / f64::from(decisive.max(1)),
                100.0 * interval.low,
                100.0 * interval.high,
                outcome.points[bot] as f64 / f64::from(total.max(1)),
            );
        }
    }

    let sum = |counts: [u32; 2]| counts[0] + counts[1];
    println!(
        "results: {} knocks, {} undercuts, {} gins, {} big gins, {} dead hands",
        sum(outcome.knocks),
        sum(outcome.undercuts),
        sum(outcome.gins),
        sum(outcome.big_gins),
        outcome.dead,
    );
    println!(
        "by bot: p1 {} knocks/{} undercuts/{} gins/{} big gins; p2 {} knocks/{} undercuts/{} gins/{} big gins",
        outcome.knocks[0],
        outcome.undercuts[0],
        outcome.gins[0],
        outcome.big_gins[0],
        outcome.knocks[1],
        outcome.undercuts[1],
        outcome.gins[1],
        outcome.big_gins[1],
    );
    print_significance(outcome, config, denominator);
}

fn json_string(value: &str) -> String {
    let mut json = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\u{08}' => json.push_str("\\b"),
            '\u{0c}' => json.push_str("\\f"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            control if control <= '\u{1f}' => {
                write!(&mut json, "\\u{:04x}", u32::from(control))
                    .expect("writing to a String cannot fail");
            }
            other => json.push(other),
        }
    }
    json.push('"');
    json
}

fn optional_json_number(value: Option<f64>) -> String {
    value.map_or_else(|| "null".into(), |number| number.to_string())
}

fn optional_json_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".into(), json_string)
}

fn rules_json(rules: &Rules) -> String {
    let oklahoma = match rules.oklahoma {
        None => "null".into(),
        Some(OklahomaAce::One) => json_string("one"),
        Some(OklahomaAce::GinOnly) => json_string("gin_only"),
        Some(_) => json_string("unknown"),
    };
    let big_gin = rules
        .big_gin_bonus
        .map_or_else(|| "null".into(), |bonus| bonus.to_string());
    let shutout = match rules.shutout {
        Shutout::Double => "{\"kind\":\"double\"}".into(),
        Shutout::Flat(bonus) => format!("{{\"kind\":\"flat\",\"bonus\":{bonus}}}"),
        _ => "{\"kind\":\"unknown\"}".into(),
    };
    format!(
        "{{\"knock_limit\":{},\"oklahoma\":{oklahoma},\"gin_bonus\":{},\"big_gin_bonus\":{big_gin},\"undercut_bonus\":{},\"undercut_on_tie\":{},\"box_bonus\":{},\"immediate_boxes\":{},\"game_bonus\":{},\"game_target\":{},\"shutout\":{shutout}}}",
        rules.knock_limit,
        rules.gin_bonus,
        rules.undercut_bonus,
        rules.undercut_on_tie,
        rules.box_bonus,
        rules.immediate_boxes,
        rules.game_bonus,
        rules.game_target,
    )
}

/// Fully expand a bot spec so a JSON result remains interpretable if defaults
/// move in a later release.
fn bot_configuration_json(spec: &str) -> String {
    let configuration = if spec == "greedy" {
        "{\"kind\":\"HeuristicBot\",\"knock_threshold\":4,\"safety_weight\":1,\"score_awareness\":40}".to_owned()
    } else if spec == "eaai" {
        "{\"kind\":\"EaaiSimpleBot\",\"draw\":\"take_only_into_immediate_meld\",\"discard_ties\":\"seeded_uniform\",\"knock\":\"first_legal\"}".to_owned()
    } else if spec == "gold-paper" {
        "{\"kind\":\"GoldPaperBot\",\"draw\":\"strict_minimum_deadwood_improvement\",\"ordinary_ties\":\"highest_pip_then_RLCard_S_H_D_C\",\"knock_ties\":\"RLCard_S_H_D_C\",\"knock_threshold\":10,\"score_or_history_dependent\":false}".to_owned()
    } else if spec == "marjj-v5-surrogate" {
        "{\"kind\":\"MarjjV5Surrogate\",\"initial_future_weight\":18,\"discount\":0.9,\"future_cards\":7,\"tie_rng\":\"arena_seeded_StdRng\",\"canonical_card_order\":\"C_H_S_D_then_rank\",\"canonical_meld_order\":\"meld_bitset\"}".to_owned()
    } else {
        let (kind, samples) = spec.split_once(':').map_or((spec, 128), |(kind, samples)| {
            (kind, samples.parse::<u32>().unwrap_or(128))
        });
        let game_value = if kind == "mca" { "affine" } else { "table" };
        format!(
            "{{\"kind\":\"MonteCarloBot\",\"samples\":{samples},\"rollout_knock_self\":255,\"rollout_knock_opponent\":255,\"opponent_model\":\"eager\",\"gate_z\":2.0,\"max_candidates\":4,\"opponent_strength_percent\":100,\"game_value\":{}}}",
            json_string(game_value),
        )
    };
    format!(
        "{{\"spec\":{},\"configuration\":{configuration}}}",
        json_string(spec),
    )
}

fn command_stdout(root: &Path, program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn sha256_file(root: &Path, relative: &str) -> Option<String> {
    command_stdout(root, "sha256sum", &[relative])?
        .split_whitespace()
        .next()
        .map(str::to_owned)
}

/// Digest the source files that determine arena behavior, including dirty
/// and untracked benchmark adapters.  Paths and lengths delimit the byte
/// stream so different file partitions cannot collide trivially.
fn source_sha256(root: &Path) -> Option<(String, usize)> {
    let listed = command_stdout(
        root,
        "git",
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "Cargo.toml",
            "src",
            "examples/arena.rs",
            "examples/strong_report.rs",
            "examples/support",
            "scripts/bench-strong.sh",
        ],
    )?;
    let mut files = listed.lines().map(str::to_owned).collect::<Vec<_>>();
    files.sort_unstable();
    files.dedup();

    let mut child = Command::new("sha256sum")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    {
        let input = child.stdin.as_mut()?;
        for relative in &files {
            let contents = std::fs::read(root.join(relative)).ok()?;
            input
                .write_all(&(relative.len() as u64).to_le_bytes())
                .ok()?;
            input.write_all(relative.as_bytes()).ok()?;
            input
                .write_all(&(contents.len() as u64).to_le_bytes())
                .ok()?;
            input.write_all(&contents).ok()?;
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()?
        .to_owned();
    Some((digest, files.len()))
}

fn cpu_model() -> Option<String> {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|cpuinfo| {
            cpuinfo.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|rest| rest.split_once(':'))
                    .map(|(_, model)| model.trim().to_owned())
            })
        })
}

fn reproducibility_json() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let git_head = command_stdout(root, "git", &["rev-parse", "HEAD"]);
    let git_status = command_stdout(root, "git", &["status", "--porcelain"]);
    let git_dirty = git_status.as_ref().map(|status| !status.is_empty());
    let rustc = command_stdout(root, "rustc", &["-Vv"]);
    let os = command_stdout(root, "uname", &["-srvmo"])
        .unwrap_or_else(|| std::env::consts::OS.to_owned());
    let cpu = cpu_model();
    let logical_threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let cargo_lock = sha256_file(root, "Cargo.lock");
    let (source, source_files) = source_sha256(root)
        .map(|(digest, files)| (Some(digest), Some(files)))
        .unwrap_or((None, None));
    let dirty = git_dirty.map_or_else(|| "null".into(), |dirty| dirty.to_string());
    let source_files = source_files.map_or_else(|| "null".into(), |files| files.to_string());
    format!(
        "{{\"source_sha256\":{},\"source_file_count\":{source_files},\"cargo_lock_sha256\":{},\"git_head\":{},\"git_dirty\":{dirty},\"rustc_vv\":{},\"os\":{},\"arch\":{},\"cpu\":{},\"logical_threads\":{logical_threads}}}",
        optional_json_string(source.as_deref()),
        optional_json_string(cargo_lock.as_deref()),
        optional_json_string(git_head.as_deref()),
        optional_json_string(rustc.as_deref()),
        json_string(&os),
        json_string(std::env::consts::ARCH),
        optional_json_string(cpu.as_deref()),
    )
}

fn ratio_moments_json(moments: RatioMoments, interval: Option<Interval>) -> String {
    let interval = interval.map_or_else(
        || "null".into(),
        |value| format!("{{\"low\":{:.15},\"high\":{:.15}}}", value.low, value.high),
    );
    format!(
        "{{\"clusters\":{},\"numerator\":{},\"denominator\":{},\"numerator_sq\":{},\"numerator_denominator\":{},\"denominator_sq\":{},\"estimate\":{},\"cluster_ci95\":{interval}}}",
        moments.clusters,
        moments.numerator,
        moments.denominator,
        moments.numerator_sq,
        moments.numerator_denominator,
        moments.denominator_sq,
        optional_json_number(moments.estimate()),
    )
}

fn signed_moments_json(moments: SignedRatioMoments) -> String {
    let interval = moments.cluster_interval().map_or_else(
        || "null".into(),
        |value| format!("{{\"low\":{:.15},\"high\":{:.15}}}", value.low, value.high),
    );
    format!(
        "{{\"clusters\":{},\"numerator\":{},\"denominator\":{},\"numerator_sq\":{},\"numerator_denominator\":{},\"denominator_sq\":{},\"estimate\":{},\"cluster_ci95\":{interval}}}",
        moments.clusters,
        moments.numerator,
        moments.denominator,
        moments.numerator_sq,
        moments.numerator_denominator,
        moments.denominator_sq,
        optional_json_number(moments.estimate()),
    )
}

fn outcome_json(outcome: &Outcome, config: &Config, elapsed: std::time::Duration) -> String {
    let total = total_plays(outcome, config);
    let decisive = outcome.wins[0] + outcome.wins[1];
    let denominator = if config.games { total } else { decisive };
    let comparison = comparison(outcome, config, denominator);
    let throughput =
        (elapsed.as_secs_f64() > 0.0).then(|| f64::from(total) / elapsed.as_secs_f64());

    let mut players = Vec::with_capacity(2);
    for bot in 0..2 {
        let (method, interval) = rate_interval(outcome, config, bot, denominator);
        let spec = if bot == 0 { &config.p1 } else { &config.p2 };
        players.push(format!(
            "{{\"seat\":\"p{}\",\"spec\":{},\"wins\":{},\"win_rate\":{:.15},\"win_ci95\":{{\"method\":{},\"low\":{:.15},\"high\":{:.15}}},\"raw_points\":{},\"win_rate_moments\":{},\"point_rate_moments\":{},\"finishes\":{{\"knock\":{},\"undercut\":{},\"gin\":{},\"big_gin\":{}}}}}",
            bot + 1,
            json_string(spec),
            outcome.wins[bot],
            f64::from(outcome.wins[bot]) / f64::from(denominator.max(1)),
            json_string(method),
            interval.low,
            interval.high,
            outcome.points[bot],
            ratio_moments_json(outcome.rates[bot], outcome.rates[bot].cluster_interval()),
            ratio_moments_json(
                outcome.point_rates[bot],
                outcome.point_rates[bot].cluster_interval_unbounded(),
            ),
            outcome.knocks[bot],
            outcome.undercuts[bot],
            outcome.gins[bot],
            outcome.big_gins[bot],
        ));
    }

    format!(
        "{{\"trials\":{},\"plays\":{total},\"decisive\":{decisive},\"failures\":0,\"elapsed_seconds\":{:.9},\"throughput_per_second\":{},\"players\":[{}],\"round_finishes\":{{\"knock\":{},\"undercut\":{},\"gin\":{},\"big_gin\":{},\"dead\":{}}},\"comparison\":{{\"rate_difference\":{:.15},\"paired_difference_sum\":{},\"paired_difference_sq_sum\":{},\"normal_z\":{},\"infinite_z_direction\":{},\"normal_p_value\":{},\"sweeps\":[{},{}],\"exact_sign_p_value\":{},\"primary_test\":{},\"primary_p_value\":{},\"point_margin_moments\":{}}}}}",
        outcome.trials,
        elapsed.as_secs_f64(),
        optional_json_number(throughput),
        players.join(","),
        outcome.knocks[0] + outcome.knocks[1],
        outcome.undercuts[0] + outcome.undercuts[1],
        outcome.gins[0] + outcome.gins[1],
        outcome.big_gins[0] + outcome.big_gins[1],
        outcome.dead,
        comparison.difference,
        outcome.d_sum,
        outcome.d2_sum,
        optional_json_number(comparison.normal_z),
        comparison.infinite_z_direction,
        optional_json_number(comparison.normal_p),
        outcome.sweeps[0],
        outcome.sweeps[1],
        optional_json_number(comparison.exact_p),
        json_string(if config.paired {
            "exact_sweep_sign"
        } else {
            "exact_sign"
        }),
        optional_json_number(comparison.exact_p),
        signed_moments_json(outcome.margin_rate),
    )
}

fn json_report(
    runs: &[SeedRun],
    pooled: &Outcome,
    config: &Config,
    elapsed: std::time::Duration,
) -> String {
    let rotation = if config.alternate_dealer {
        "alternate_after_scored_round"
    } else {
        "winner_deals"
    };
    let mode = if config.games { "games" } else { "rounds" };
    let points_metric = if config.games {
        "raw_game_score"
    } else {
        "round_points"
    };
    let seeds = runs
        .iter()
        .map(|run| run.seed.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let per_seed = runs
        .iter()
        .map(|run| {
            format!(
                "{{\"seed\":{},\"outcome\":{}}}",
                run.seed,
                outcome_json(&run.outcome, config, run.elapsed),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"schema\":\"gin-rummy-arena/v1\",\"mode\":{},\"paired\":{},\"count_per_seed\":{},\"failures\":0,\"rules\":{{\"preset\":{},\"values\":{}}},\"dealer_rotation\":{},\"points_metric\":{},\"bots\":{{\"p1\":{},\"p2\":{}}},\"seeds\":[{seeds}],\"reproducibility\":{},\"runs\":[{per_seed}],\"pooled\":{}}}",
        json_string(mode),
        config.paired,
        config.count,
        json_string(config.rules_name),
        rules_json(&config.rules),
        json_string(rotation),
        json_string(points_metric),
        bot_configuration_json(&config.p1),
        bot_configuration_json(&config.p2),
        reproducibility_json(),
        outcome_json(pooled, config, elapsed),
    )
}

fn print_json_report(
    runs: &[SeedRun],
    pooled: &Outcome,
    config: &Config,
    elapsed: std::time::Duration,
) {
    println!("{}", json_report(runs, pooled, config, elapsed));
}

fn main() -> Result<()> {
    let mut config = parse_args()?;
    if config.seeds.is_empty() {
        config.seeds.push(rand::rng().random());
    }

    let start = Instant::now();
    let runs = config
        .seeds
        .iter()
        .copied()
        .map(|seed| run_seed(&config, seed))
        .collect::<Result<Vec<_>>>()?;
    let pooled = runs.iter().fold(Outcome::default(), |all, run| {
        all.merge(run.outcome.clone())
    });
    let elapsed = start.elapsed();

    match config.format {
        OutputFormat::Text if runs.len() == 1 => {
            print_text_outcome(
                &runs[0].outcome,
                &config,
                runs[0].elapsed,
                &format!("seed {}", runs[0].seed),
            );
        }
        OutputFormat::Text => {
            for run in &runs {
                print_text_outcome(
                    &run.outcome,
                    &config,
                    run.elapsed,
                    &format!("seed {}", run.seed),
                );
                println!();
            }
            print_text_outcome(
                &pooled,
                &config,
                elapsed,
                &format!("pooled {} seeds", runs.len()),
            );
        }
        OutputFormat::Json => print_json_report(&runs, &pooled, &config, elapsed),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args<'a>(values: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
        values.iter().map(|value| (*value).to_owned())
    }

    #[test]
    fn parses_multiple_seeds_and_json() {
        let config = parse_args_from(args(&[
            "--rounds", "12", "--seeds", "7,8", "--format", "json",
        ]))
        .expect("valid arena arguments");
        assert_eq!(config.count, 12);
        assert_eq!(config.seeds, [7, 8]);
        assert_eq!(config.format, OutputFormat::Json);
    }

    #[test]
    fn repeated_seed_is_rejected() {
        let error = parse_args_from(args(&["--seed", "7", "--seeds", "8,7"]))
            .err()
            .expect("a duplicate seed is invalid");
        assert!(error.to_string().contains("duplicate seed 7"));
    }

    #[test]
    fn json_strings_escape_control_characters() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
        assert_eq!(json_string("\u{01}"), "\"\\u0001\"");
    }

    #[test]
    fn bare_mc_json_expands_the_real_default_configuration() {
        let config = bot_configuration_json("mc");
        let value: serde_json::Value = serde_json::from_str(&config).expect("valid bot JSON");
        assert_eq!(value["spec"], "mc");
        assert_eq!(value["configuration"]["samples"], 128);
        assert_eq!(value["configuration"]["game_value"], "table");
    }

    #[test]
    fn seeded_multi_seed_json_is_deterministic_after_timing_normalization() {
        let config = Config {
            count: 2,
            games: false,
            paired: true,
            alternate_dealer: true,
            p1: "greedy".into(),
            p2: "gold-paper".into(),
            seeds: vec![7, 8],
            rules: eaai_rules(),
            rules_name: "eaai",
            format: OutputFormat::Json,
        };
        let run = || {
            config
                .seeds
                .iter()
                .copied()
                .map(|seed| run_seed(&config, seed))
                .collect::<Result<Vec<_>>>()
                .expect("seeded fixture plays")
        };
        let normalize = |mut runs: Vec<SeedRun>| {
            for run in &mut runs {
                run.elapsed = std::time::Duration::ZERO;
            }
            let pooled = runs.iter().fold(Outcome::default(), |all, run| {
                all.merge(run.outcome.clone())
            });
            json_report(&runs, &pooled, &config, std::time::Duration::ZERO)
        };
        let first = normalize(run());
        let second = normalize(run());
        assert_eq!(first, second);
        let parsed: serde_json::Value = serde_json::from_str(&first).expect("valid report JSON");
        assert_eq!(parsed["schema"], "gin-rummy-arena/v1");
        assert_eq!(parsed["seeds"], serde_json::json!([7, 8]));
    }

    #[test]
    fn dead_hands_keep_the_dealer_in_challenge_games() {
        let dealer = Player::One;
        assert_eq!(challenge_next_dealer(dealer, RoundResult::Dead), dealer);
        assert_eq!(
            challenge_next_dealer(
                dealer,
                RoundResult::Gin {
                    winner: Player::One,
                    deadwood: 10,
                },
            ),
            dealer.opponent(),
        );
    }
}
