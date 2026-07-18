//! Tune a bot's knobs by whole-game A/B self-play.
//!
//! ```console
//! cargo run --release --example tune -- --games 20000 --seed 1 \
//!   --knock 4,6,8,10 --awareness 0,4,8,16,32
//! cargo run --release --example tune -- --games 2000 --seed 1 --rules eaai \
//!   --opponent eaai --mc-samples 64 --opp-model eager,meld
//! ```
//!
//! Each arm is a candidate configuration: by default a `HeuristicConfig`
//! `(knock_threshold, score_awareness)` pair; with `--mc-samples N` a
//! Monte Carlo [`McConfig`], whose swept knobs are `--rollout-knock`
//! (the bot's own continuation), `--opp-knock` (the modeled opponent's),
//! `--opp-model eager,meld`, `--gate`, `--max-candidates`, and
//! `--opp-strength` (percent).  Arms are the cartesian product of the
//! given lists — sweep one list at a time or the arm count multiplies.
//! Score-aware knobs only show up across a game (a single round has no
//! scoreboard), so evaluation is game-based, not round-based like
//! `arena`.  Every arm replays the *same* seeded deals against the same
//! opponent (common random numbers), so the arms are paired and directly
//! comparable, and the printed table is sorted by game-win rate.  Each
//! arm's games are seeded by index and played in parallel across the
//! CPUs, so the win counts stay deterministic however the work
//! schedules.  Never build with `--features parallel`: in-decision
//! parallelism would fight the game-level fan-out for the same rayon
//! pool.
//!
//! Comparing many arms on one seed and keeping the maximum overstates the
//! winner.  Search on one seed, then re-confirm the single best arm on
//! another before trusting it:
//!
//! ```console
//! cargo run --release --example tune -- --games 20000 --seed 2 \
//!   --knock 6 --awareness 8
//! ```

use anyhow::{Context as _, Result, bail};
use gin_rummy::{Game, Player, Rules};
use gin_rummy_engine::{
    EaaiSimpleBot, HeuristicBot, HeuristicConfig, McConfig, MonteCarloBot, OpponentModel, Strategy,
    play_game,
};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rayon::prelude::*;
use std::time::Instant;

/// Offsets putting the opponent's and a Monte Carlo candidate's
/// generators on streams of their own, so every arm plays game `i` on
/// the same deal against the same opponent.
const OPPONENT_STREAM: u64 = 0xD1B5_4A32_D192_ED03;
const CANDIDATE_STREAM: u64 = 0x94D0_49BB_1331_11EB;

/// The fixed opponent every candidate is scored against
#[derive(Clone)]
enum Opponent {
    /// A heuristic bot with a given `(knock_threshold, score_awareness)`
    Greedy(u8, u8),
    /// A Monte Carlo bot with a given sample count
    Mc(u32),
    /// The EAAI-2021 challenge baseline
    Eaai,
}

/// One candidate configuration under evaluation
#[derive(Clone, Copy)]
enum Arm {
    /// A `HeuristicConfig` knock-policy pair
    Heuristic(u8, u8),
    /// A full Monte Carlo configuration
    Mc(McConfig),
}

impl Arm {
    /// Render the swept knobs for the results table.
    fn label(&self) -> String {
        match *self {
            Self::Heuristic(knock, awareness) => {
                format!("knock={knock:>2} awareness={awareness:>3}")
            }
            Self::Mc(config) => format!(
                "mc:{} sknock={:>3} oknock={:>3} model={} gate={} cand={} strength={}%",
                config.samples,
                config.rollout_knock_self,
                config.rollout_knock_opponent,
                match config.opponent_model {
                    OpponentModel::MeldOnly => "meld",
                    _ => "eager",
                },
                config.gate_z,
                config.max_candidates,
                config.opponent_strength_percent,
            ),
        }
    }
}

struct Config {
    games: u32,
    seed: u64,
    knocks: Vec<u8>,
    awareness: Vec<u8>,
    mc_samples: Option<u32>,
    rollout_knock: Vec<u8>,
    opp_knock: Vec<u8>,
    opp_model: Vec<OpponentModel>,
    gate: Vec<f64>,
    max_candidates: Vec<usize>,
    opp_strength: Vec<u32>,
    opponent: Opponent,
    rules: Rules,
}

/// Build a fresh candidate `HeuristicBot` from a swept knob pair.
fn heuristic(knock: u8, awareness: u8) -> HeuristicBot {
    // `HeuristicConfig` is non-exhaustive, so start from Default and adjust.
    let mut cfg = HeuristicConfig::default();
    cfg.knock_threshold = knock;
    cfg.score_awareness = awareness;
    HeuristicBot::with_config(cfg)
}

/// Build the candidate bot for one arm, seeding any Monte Carlo candidate
/// deterministically per game.
fn candidate(arm: Arm, seed: u64) -> Box<dyn Strategy> {
    match arm {
        Arm::Heuristic(knock, awareness) => Box::new(heuristic(knock, awareness)),
        Arm::Mc(config) => Box::new(MonteCarloBot::with_config(
            StdRng::seed_from_u64(seed),
            config,
        )),
    }
}

/// Build the fixed opponent, seeding any randomized bot deterministically
/// so every arm faces the same opponent on the same deals.
fn opponent(spec: &Opponent, seed: u64) -> Box<dyn Strategy> {
    match *spec {
        Opponent::Greedy(knock, awareness) => Box::new(heuristic(knock, awareness)),
        Opponent::Mc(samples) => {
            Box::new(MonteCarloBot::new(StdRng::seed_from_u64(seed)).samples(samples))
        }
        Opponent::Eaai => Box::new(EaaiSimpleBot::new(StdRng::seed_from_u64(seed))),
    }
}

/// Parse an opponent spec: `greedy`, `greedy:KNOCK:AWARENESS`, `mc[:N]`,
/// or `eaai`.
fn parse_opponent(spec: &str) -> Result<Opponent> {
    let mut parts = spec.split(':');
    match parts.next() {
        // The shipped default bot.
        Some("greedy") => match (parts.next(), parts.next()) {
            (None, _) => Ok(Opponent::Greedy(
                HeuristicConfig::default().knock_threshold,
                HeuristicConfig::default().score_awareness,
            )),
            (Some(knock), Some(awareness)) => {
                Ok(Opponent::Greedy(knock.parse()?, awareness.parse()?))
            }
            (Some(_), None) => {
                bail!("greedy opponent needs both knock and awareness, e.g. greedy:2:8")
            }
        },
        Some("mc") => Ok(Opponent::Mc(parts.next().map_or(Ok(64), str::parse)?)),
        Some("eaai") => Ok(Opponent::Eaai),
        _ => bail!(
            "unknown opponent {spec:?} (greedy | greedy:knock:awareness | mc[:samples] | eaai)"
        ),
    }
}

/// Parse a comma-separated list, e.g. `4,6,8`.
fn parse_list<T: std::str::FromStr>(text: &str) -> Result<Vec<T>>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    text.split(',')
        .map(|item| item.trim().parse().map_err(anyhow::Error::from))
        .collect()
}

/// Parse a comma-separated list of opponent models: `eager,meld`.
fn parse_models(text: &str) -> Result<Vec<OpponentModel>> {
    text.split(',')
        .map(|item| match item.trim() {
            "eager" => Ok(OpponentModel::Eager),
            "meld" => Ok(OpponentModel::MeldOnly),
            other => bail!("unknown opponent model {other:?} (eager | meld)"),
        })
        .collect()
}

fn parse_args() -> Result<Config> {
    let defaults = McConfig::default();
    let mut config = Config {
        games: 2000,
        seed: 1,
        knocks: vec![10],
        awareness: vec![0, 4, 8, 16, 32],
        mc_samples: None,
        rollout_knock: vec![defaults.rollout_knock_self],
        opp_knock: vec![defaults.rollout_knock_opponent],
        opp_model: vec![defaults.opponent_model],
        gate: vec![defaults.gate_z],
        max_candidates: vec![defaults.max_candidates],
        opp_strength: vec![defaults.opponent_strength_percent],
        opponent: Opponent::Greedy(
            HeuristicConfig::default().knock_threshold,
            HeuristicConfig::default().score_awareness,
        ),
        rules: Rules::default(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--games" => config.games = value()?.parse()?,
            "--seed" => config.seed = value()?.parse()?,
            "--knock" => config.knocks = parse_list(&value()?)?,
            "--awareness" => config.awareness = parse_list(&value()?)?,
            "--mc-samples" => config.mc_samples = Some(value()?.parse()?),
            "--rollout-knock" => config.rollout_knock = parse_list(&value()?)?,
            "--opp-knock" => config.opp_knock = parse_list(&value()?)?,
            "--opp-model" => config.opp_model = parse_models(&value()?)?,
            "--gate" => config.gate = parse_list(&value()?)?,
            "--max-candidates" => config.max_candidates = parse_list(&value()?)?,
            "--opp-strength" => config.opp_strength = parse_list(&value()?)?,
            "--opponent" => config.opponent = parse_opponent(&value()?)?,
            "--rules" => {
                config.rules = match value()?.as_str() {
                    "modern" => Rules::new(),
                    "classic" => Rules::classic(),
                    "palace" => Rules::palace(),
                    // The EAAI-2021 challenge: modern bonuses, no big gin.
                    "eaai" => {
                        let mut rules = Rules::new();
                        rules.big_gin_bonus = None;
                        rules
                    }
                    other => bail!("unknown rules preset {other:?}"),
                }
            }
            // Override the per-hand box bonus on top of the chosen preset,
            // to probe how the knock policy tracks the scoring.
            "--box-bonus" => config.rules.box_bonus = value()?.parse()?,
            other => bail!(
                "unknown flag {other:?} \
                 (--games/--seed/--knock/--awareness/--mc-samples/--rollout-knock/\
                 --opp-knock/--opp-model/--gate/--max-candidates/--opp-strength/\
                 --opponent/--rules/--box-bonus)"
            ),
        }
    }
    Ok(config)
}

/// The arms this run sweeps: heuristic knock pairs by default, the
/// cartesian product of the Monte Carlo lists under `--mc-samples`.
fn arms(config: &Config) -> Vec<Arm> {
    let Some(samples) = config.mc_samples else {
        return config
            .knocks
            .iter()
            .flat_map(|&knock| {
                config
                    .awareness
                    .iter()
                    .map(move |&awareness| Arm::Heuristic(knock, awareness))
            })
            .collect();
    };

    let mut arms = Vec::new();
    for &rollout_knock_self in &config.rollout_knock {
        for &rollout_knock_opponent in &config.opp_knock {
            for &opponent_model in &config.opp_model {
                for &gate_z in &config.gate {
                    for &max_candidates in &config.max_candidates {
                        for &opponent_strength_percent in &config.opp_strength {
                            // `McConfig` is non-exhaustive: start from
                            // Default and adjust.
                            let mut mc = McConfig::default();
                            mc.samples = samples;
                            mc.rollout_knock_self = rollout_knock_self;
                            mc.rollout_knock_opponent = rollout_knock_opponent;
                            mc.opponent_model = opponent_model;
                            mc.gate_z = gate_z;
                            mc.max_candidates = max_candidates;
                            mc.opponent_strength_percent = opponent_strength_percent;
                            arms.push(Arm::Mc(mc));
                        }
                    }
                }
            }
        }
    }
    arms
}

/// The 95% Wilson score interval for `wins` out of `n` decisive trials.
// ponytail: copied from arena.rs — 12 lines not worth a shared example crate.
fn wilson(wins: u32, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (w, n) = (f64::from(wins), f64::from(n));
    let z = 1.96;
    let p = w / n;
    let denom = 1.0 + z * z / n;
    let center = (p + z * z / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denom;
    (center - half, center + half)
}

/// Play game `index` of an arm and report whether the candidate won.
///
/// The deal, the opponent, and a randomized candidate are seeded from
/// `index` alone, not from the arm's knobs, so every arm plays game
/// `index` on the same deal against the same opponent — the common
/// random numbers that pair the arms — regardless of the order the games
/// run in.
fn play_one(arm: Arm, config: &Config, index: u32) -> Result<bool> {
    // SplitMix64 constant: mix the index so adjacent games get
    // decorrelated seeds.
    let mixed = u64::from(index).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut rng = StdRng::seed_from_u64(config.seed ^ mixed);
    let mut candidate = candidate(arm, config.seed ^ mixed ^ CANDIDATE_STREAM);
    let mut opponent = opponent(&config.opponent, config.seed ^ mixed ^ OPPONENT_STREAM);

    // Swap seats every game and alternate the dealer so neither is favoured.
    let swapped = index % 2 == 1;
    let candidate_seat = usize::from(swapped);
    let dealer = if index % 4 < 2 {
        Player::One
    } else {
        Player::Two
    };
    let seats: [&mut dyn Strategy; 2] = if swapped {
        [&mut *opponent, &mut *candidate]
    } else {
        [&mut *candidate, &mut *opponent]
    };
    let mut game = Game::new(config.rules, dealer);
    let score = play_game(&mut game, seats, &mut rng)?;
    Ok(score.winner as usize == candidate_seat)
}

/// The candidate's game wins against the fixed opponent over `games` whole
/// games.
///
/// Games are seeded per index (see [`play_one`]) and played in parallel
/// across the available CPUs; every arm still plays game `i` on the same
/// deal against the same opponent, so the arms stay paired and the win
/// count does not depend on how the work is scheduled.
fn evaluate(arm: Arm, config: &Config) -> Result<u32> {
    // Per-game seeding makes the games independent, so rayon can fan them
    // across the pool; summing an associative reduce keeps the count
    // deterministic however the work splits.
    (0..config.games)
        .into_par_iter()
        .map(|index| play_one(arm, config, index).map(u32::from))
        .try_reduce(|| 0, |a, b| Ok(a + b))
}

fn main() -> Result<()> {
    let config = parse_args()?;
    let start = Instant::now();

    // Each arm is streamed to stderr as it finishes, so a long overnight
    // run under `idle-run` shows progress; the sorted table lands on
    // stdout at the end.
    let arms = arms(&config);
    let mut results: Vec<(Arm, u32)> = Vec::new();
    for &arm in &arms {
        let wins = evaluate(arm, &config)?;
        eprintln!(
            "  arm {}/{}: {} -> {wins}/{}",
            results.len() + 1,
            arms.len(),
            arm.label(),
            config.games,
        );
        results.push((arm, wins));
    }

    let elapsed = start.elapsed();
    let total = config.games * results.len() as u32;
    let versus = match config.opponent {
        Opponent::Greedy(knock, awareness) => {
            format!("greedy(knock={knock}, awareness={awareness})")
        }
        Opponent::Mc(samples) => format!("mc:{samples}"),
        Opponent::Eaai => "eaai".to_string(),
    };
    println!(
        "{total} games in {:.1?} ({:.0} games/s) vs {versus}",
        elapsed,
        f64::from(total) / elapsed.as_secs_f64(),
    );

    // Best game-win rate first.
    results.sort_by_key(|result| std::cmp::Reverse(result.1));
    for (arm, wins) in results {
        let (lo, hi) = wilson(wins, config.games);
        println!(
            "{}: {wins}/{} ({:.1}%, 95% CI {:.1}%\u{2013}{:.1}%)",
            arm.label(),
            config.games,
            100.0 * f64::from(wins) / f64::from(config.games.max(1)),
            100.0 * lo,
            100.0 * hi,
        );
    }
    Ok(())
}
