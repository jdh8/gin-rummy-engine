//! Bot-vs-bot tournaments with win-rate statistics.
//!
//! ```console
//! cargo run --release --example arena -- --rounds 1000 --p1 greedy --p2 mc:32
//! cargo run --release --example arena -- --games 100 --p1 mc:16 --p2 mc:64 --seed 7
//! ```
//!
//! Seats and the dealer alternate every trial, so neither bot benefits from
//! going first.  `--rounds` plays independent single rounds; `--games`
//! plays whole games to the target score.

use anyhow::{Context as _, Result, bail};
use gin_rummy::{Game, Player, RoundResult, Rules};
use gin_rummy_engine::{HeuristicBot, MonteCarloBot, Strategy, Table, play_game};
use rand::rngs::StdRng;
use rand::{RngExt as _, SeedableRng};
use std::time::Instant;

struct Config {
    count: u32,
    games: bool,
    p1: String,
    p2: String,
    seed: Option<u64>,
    rules: Rules,
}

fn parse_args() -> Result<Config> {
    let mut config = Config {
        count: 200,
        games: false,
        p1: "greedy".into(),
        p2: "mc:32".into(),
        seed: None,
        rules: Rules::default(),
    };
    let mut args = std::env::args().skip(1);
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
            "--p1" => config.p1 = value()?,
            "--p2" => config.p2 = value()?,
            "--seed" => config.seed = Some(value()?.parse()?),
            "--rules" => {
                config.rules = match value()?.as_str() {
                    "modern" => Rules::new(),
                    "classic" => Rules::classic(),
                    "palace" => Rules::palace(),
                    other => bail!("unknown rules preset {other:?}"),
                }
            }
            other => bail!("unknown flag {other:?} (--rounds/--games/--p1/--p2/--seed/--rules)"),
        }
    }
    Ok(config)
}

fn make_bot(spec: &str, rng: &mut StdRng) -> Result<Box<dyn Strategy>> {
    let (kind, samples) = match spec.split_once(':') {
        Some((kind, samples)) => (kind, Some(samples.parse::<u32>()?)),
        None => (spec, None),
    };
    match kind {
        "greedy" => Ok(Box::new(HeuristicBot::new())),
        "mc" => Ok(Box::new(
            MonteCarloBot::new(StdRng::seed_from_u64(rng.random())).samples(samples.unwrap_or(32)),
        )),
        other => bail!("unknown bot {other:?} (greedy | mc[:samples])"),
    }
}

#[derive(Default)]
struct Tally {
    wins: [u32; 2],
    points: [u64; 2],
    knocks: u32,
    undercuts: u32,
    gins: u32,
    big_gins: u32,
    dead: u32,
}

impl Tally {
    /// Record a round result; `bot_of_seat` maps each seat to a bot index.
    fn record(&mut self, result: RoundResult, rules: &Rules, bot_of_seat: [usize; 2]) {
        match result {
            RoundResult::Dead => self.dead += 1,
            RoundResult::Knock { .. } => self.knocks += 1,
            RoundResult::Undercut { .. } => self.undercuts += 1,
            RoundResult::Gin { .. } => self.gins += 1,
            RoundResult::BigGin { .. } => self.big_gins += 1,
            _ => {}
        }
        if let Some(winner) = result.winner() {
            let bot = bot_of_seat[winner as usize];
            self.wins[bot] += 1;
            self.points[bot] += u64::from(result.points(rules));
        }
    }
}

/// The 95% Wilson score interval for `wins` out of `n` decisive trials
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

fn main() -> Result<()> {
    let config = parse_args()?;
    let mut rng = match config.seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => StdRng::from_rng(&mut rand::rng()),
    };
    let mut bots = [
        make_bot(&config.p1, &mut rng)?,
        make_bot(&config.p2, &mut rng)?,
    ];
    let names = [format!("p1={}", config.p1), format!("p2={}", config.p2)];

    let mut tally = Tally::default();
    let mut game_wins = [0u32; 2];
    let start = Instant::now();

    for index in 0..config.count {
        // Swap seats every trial and alternate the dealer, cancelling any
        // first-move advantage.
        let swapped = index % 2 == 1;
        let bot_of_seat = if swapped { [1, 0] } else { [0, 1] };
        let dealer = if index % 4 < 2 {
            Player::One
        } else {
            Player::Two
        };
        let [one, two] = &mut bots;
        let seats: [&mut dyn Strategy; 2] = if swapped {
            [&mut **two, &mut **one]
        } else {
            [&mut **one, &mut **two]
        };

        if config.games {
            let mut game = Game::new(config.rules, dealer);
            let score = play_game(&mut game, seats, &mut rng)?;
            let bot = bot_of_seat[score.winner as usize];
            game_wins[bot] += 1;
            tally.points[bot] += u64::from(score.totals[score.winner as usize]);
        } else {
            let mut table = Table::deal(config.rules, dealer, &mut rng);
            let result = table.play(seats)?;
            tally.record(result, &config.rules, bot_of_seat);
        }
    }

    let elapsed = start.elapsed();
    let unit = if config.games { "game" } else { "round" };
    println!(
        "{} {unit}s in {:.2?} ({:.1} {unit}s/s)",
        config.count,
        elapsed,
        f64::from(config.count) / elapsed.as_secs_f64(),
    );

    if config.games {
        for (bot, name) in names.iter().enumerate() {
            let (lo, hi) = wilson(game_wins[bot], config.count);
            println!(
                "{name}: {} game wins / {} ({:.1}%, 95% CI {:.1}%\u{2013}{:.1}%), {} winning points",
                game_wins[bot],
                config.count,
                100.0 * f64::from(game_wins[bot]) / f64::from(config.count.max(1)),
                100.0 * lo,
                100.0 * hi,
                tally.points[bot],
            );
        }
    } else {
        let decisive = tally.wins[0] + tally.wins[1];
        for (bot, name) in names.iter().enumerate() {
            let (lo, hi) = wilson(tally.wins[bot], decisive);
            println!(
                "{name}: {} wins / {} decisive ({:.1}%, 95% CI {:.1}%\u{2013}{:.1}%), {:.2} points/round",
                tally.wins[bot],
                decisive,
                100.0 * f64::from(tally.wins[bot]) / f64::from(decisive.max(1)),
                100.0 * lo,
                100.0 * hi,
                tally.points[bot] as f64 / f64::from(config.count.max(1)),
            );
        }
        println!(
            "results: {} knocks, {} undercuts, {} gins, {} big gins, {} dead hands",
            tally.knocks, tally.undercuts, tally.gins, tally.big_gins, tally.dead,
        );
    }
    Ok(())
}
