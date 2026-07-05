//! Tune `HeuristicBot`'s score-aware knock policy by self-play.
//!
//! ```console
//! cargo run --release --example tune -- --games 20000 --seed 1 \
//!   --knock 4,6,8,10 --awareness 0,4,8,16,32
//! ```
//!
//! Each arm is a candidate `HeuristicConfig` — a `(knock_threshold,
//! score_awareness)` pair — played over whole games against the current
//! default [`HeuristicBot`].  Score-awareness only shows up across a game
//! (a single round has no scoreboard), so evaluation is game-based, not
//! round-based like `arena`.  Every arm replays the *same* seeded deals
//! (common random numbers), so the arms are paired and directly
//! comparable, and the printed table is sorted by game-win rate.  Each
//! arm's games are seeded by index and played in parallel across the
//! CPUs, so the win counts stay deterministic however the work schedules.
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
use gin_rummy_engine::{HeuristicBot, HeuristicConfig, MonteCarloBot, Strategy, play_game};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rayon::prelude::*;
use std::time::Instant;

/// The fixed opponent every candidate is scored against
#[derive(Clone)]
enum Opponent {
    /// A heuristic bot with a given `(knock_threshold, score_awareness)`
    Greedy(u8, u8),
    /// A Monte Carlo bot with a given sample count
    Mc(u32),
}

struct Config {
    games: u32,
    seed: u64,
    knocks: Vec<u8>,
    awareness: Vec<u8>,
    opponent: Opponent,
    rules: Rules,
}

/// Build a fresh candidate `HeuristicBot` from a swept knob pair.
fn candidate(knock: u8, awareness: u8) -> HeuristicBot {
    // `HeuristicConfig` is non-exhaustive, so start from Default and adjust.
    let mut cfg = HeuristicConfig::default();
    cfg.knock_threshold = knock;
    cfg.score_awareness = awareness;
    HeuristicBot::with_config(cfg)
}

/// Build the fixed opponent, seeding any Monte Carlo bot deterministically
/// so every arm faces the same opponent on the same deals.
fn opponent(spec: &Opponent, seed: u64) -> Box<dyn Strategy> {
    match *spec {
        Opponent::Greedy(knock, awareness) => Box::new(candidate(knock, awareness)),
        Opponent::Mc(samples) => {
            Box::new(MonteCarloBot::new(StdRng::seed_from_u64(seed)).samples(samples))
        }
    }
}

/// Parse an opponent spec: `greedy`, `greedy:KNOCK:AWARENESS`, or `mc[:N]`.
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
        _ => bail!("unknown opponent {spec:?} (greedy | greedy:knock:awareness | mc[:samples])"),
    }
}

/// Parse a comma-separated list of small integers, e.g. `4,6,8`.
fn parse_list(text: &str) -> Result<Vec<u8>> {
    text.split(',')
        .map(|item| item.trim().parse().map_err(anyhow::Error::from))
        .collect()
}

fn parse_args() -> Result<Config> {
    let mut config = Config {
        games: 2000,
        seed: 1,
        knocks: vec![10],
        awareness: vec![0, 4, 8, 16, 32],
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
            "--opponent" => config.opponent = parse_opponent(&value()?)?,
            "--rules" => {
                config.rules = match value()?.as_str() {
                    "modern" => Rules::new(),
                    "classic" => Rules::classic(),
                    "palace" => Rules::palace(),
                    other => bail!("unknown rules preset {other:?}"),
                }
            }
            // Override the per-hand box bonus on top of the chosen preset,
            // to probe how the knock policy tracks the scoring.
            "--box-bonus" => config.rules.box_bonus = value()?.parse()?,
            other => bail!(
                "unknown flag {other:?} \
                 (--games/--seed/--knock/--awareness/--opponent/--rules/--box-bonus)"
            ),
        }
    }
    Ok(config)
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
/// The deal and the Monte Carlo opponent are seeded from `index` alone,
/// not from the arm's knobs, so every arm plays game `index` on the same
/// deal against the same opponent — the common random numbers that pair
/// the arms — regardless of the order the games run in.
fn play_one(knock: u8, awareness: u8, config: &Config, index: u32) -> Result<bool> {
    // SplitMix64 constants: mix the index so adjacent games get
    // decorrelated seeds, and offset the opponent onto its own stream.
    let mixed = u64::from(index).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut rng = StdRng::seed_from_u64(config.seed ^ mixed);
    let mut candidate = candidate(knock, awareness);
    let mut opponent = opponent(
        &config.opponent,
        config.seed ^ mixed ^ 0xD1B5_4A32_D192_ED03,
    );

    // Swap seats every game and alternate the dealer so neither is favoured.
    let swapped = index % 2 == 1;
    let candidate_seat = usize::from(swapped);
    let dealer = if index % 4 < 2 {
        Player::One
    } else {
        Player::Two
    };
    let seats: [&mut dyn Strategy; 2] = if swapped {
        [&mut *opponent, &mut candidate]
    } else {
        [&mut candidate, &mut *opponent]
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
fn evaluate(knock: u8, awareness: u8, config: &Config) -> Result<u32> {
    // Per-game seeding makes the games independent, so rayon can fan them
    // across the pool; summing an associative reduce keeps the count
    // deterministic however the work splits.
    (0..config.games)
        .into_par_iter()
        .map(|index| play_one(knock, awareness, config, index).map(u32::from))
        .try_reduce(|| 0, |a, b| Ok(a + b))
}

fn main() -> Result<()> {
    let config = parse_args()?;
    let start = Instant::now();

    // The candidate keeps the default safety weight; only the knock policy
    // is swept.  Each arm is streamed to stderr as it finishes, so a long
    // overnight run under `idle-run` shows progress; the sorted table lands
    // on stdout at the end.
    let mut arms: Vec<(u8, u8, u32)> = Vec::new();
    for &knock in &config.knocks {
        for &awareness in &config.awareness {
            let wins = evaluate(knock, awareness, &config)?;
            eprintln!(
                "  arm {}/{}: knock={knock} awareness={awareness} -> {wins}/{}",
                arms.len() + 1,
                config.knocks.len() * config.awareness.len(),
                config.games,
            );
            arms.push((knock, awareness, wins));
        }
    }

    let elapsed = start.elapsed();
    let total = config.games * arms.len() as u32;
    let versus = match config.opponent {
        Opponent::Greedy(knock, awareness) => {
            format!("greedy(knock={knock}, awareness={awareness})")
        }
        Opponent::Mc(samples) => format!("mc:{samples}"),
    };
    println!(
        "{total} games in {:.1?} ({:.0} games/s) vs {versus}",
        elapsed,
        f64::from(total) / elapsed.as_secs_f64(),
    );

    // Best game-win rate first.
    arms.sort_by_key(|arm| std::cmp::Reverse(arm.2));
    for (knock, awareness, wins) in arms {
        let (lo, hi) = wilson(wins, config.games);
        println!(
            "knock={knock:>2} awareness={awareness:>3}: \
             {wins}/{} ({:.1}%, 95% CI {:.1}%\u{2013}{:.1}%)",
            config.games,
            100.0 * f64::from(wins) / f64::from(config.games.max(1)),
            100.0 * lo,
            100.0 * hi,
        );
    }
    Ok(())
}
