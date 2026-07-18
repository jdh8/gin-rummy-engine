//! Bot-vs-bot tournaments with win-rate statistics.
//!
//! ```console
//! cargo run --release --example arena -- --rounds 1000 --p1 greedy --p2 mc:32
//! cargo run --release --example arena -- --games 3000 --p1 mc:64 --p2 eaai \
//!   --rules eaai --alternate-dealer --seed 7
//! ```
//!
//! Trials are seeded by index and fan out across the CPUs, so a given
//! `--seed` reproduces the same counts however the work schedules.  By
//! default every trial is a *mirrored pair*: both bots play the same
//! deal(s) once from each seat, cancelling deal luck out of the
//! comparison, and the printed significance line tests the paired
//! per-trial difference.  `--unpaired` plays a single game per trial
//! (seats alternating by index) instead.  `--rounds N` plays N trials of
//! independent single rounds; `--games N` plays N trials of whole games
//! to the target score, where `--alternate-dealer` alternates the deal
//! every hand as the EAAI challenge did, rather than gin rummy's usual
//! winner-deals-next.
//!
//! Build without `--features parallel`: Monte Carlo's in-decision
//! parallelism would compete with the trial-level fan-out for the same
//! rayon pool.

use anyhow::{Context as _, Result, bail};
use gin_rummy::{FinalScore, Game, Player, Round, RoundResult, Rules};
use gin_rummy_engine::{EaaiSimpleBot, HeuristicBot, MonteCarloBot, Strategy, Table, play_game};
use rand::rngs::StdRng;
use rand::{RngExt as _, SeedableRng};
use rayon::prelude::*;
use std::time::Instant;

/// SplitMix64's increment: mixed into the seed so adjacent trial indices
/// get decorrelated random streams.
const MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// Offsets putting each bot's generator on its own stream, independent of
/// the deal stream.  Within a mirrored pair each bot is re-seeded
/// identically in both orientations, so the deals are the only difference
/// between the two runs.
const BOT_STREAM: [u64; 2] = [0xD1B5_4A32_D192_ED03, 0x94D0_49BB_1331_11EB];

struct Config {
    count: u32,
    games: bool,
    paired: bool,
    alternate_dealer: bool,
    p1: String,
    p2: String,
    seed: Option<u64>,
    rules: Rules,
}

fn parse_args() -> Result<Config> {
    let mut config = Config {
        count: 200,
        games: false,
        paired: true,
        alternate_dealer: false,
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
            "--paired" => config.paired = true,
            "--unpaired" => config.paired = false,
            "--alternate-dealer" => config.alternate_dealer = true,
            "--p1" => config.p1 = value()?,
            "--p2" => config.p2 = value()?,
            "--seed" => config.seed = Some(value()?.parse()?),
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
            other => bail!(
                "unknown flag {other:?} (--rounds/--games/--paired/--unpaired/\
                 --alternate-dealer/--p1/--p2/--seed/--rules)"
            ),
        }
    }
    Ok(config)
}

fn make_bot(spec: &str, seed: u64) -> Result<Box<dyn Strategy>> {
    let (kind, samples) = match spec.split_once(':') {
        Some((kind, samples)) => (kind, Some(samples.parse::<u32>()?)),
        None => (spec, None),
    };
    match kind {
        "greedy" => Ok(Box::new(HeuristicBot::new())),
        // The EAAI-2021 challenge baseline, the cross-engine yardstick.
        "eaai" => Ok(Box::new(EaaiSimpleBot::new(StdRng::seed_from_u64(seed)))),
        "mc" => Ok(Box::new(
            MonteCarloBot::new(StdRng::seed_from_u64(seed)).samples(samples.unwrap_or(32)),
        )),
        other => bail!("unknown bot {other:?} (greedy | eaai | mc[:samples])"),
    }
}

/// Everything a trial reports, summable across trials in any order
#[derive(Default)]
struct Outcome {
    /// Decisive round wins (rounds mode) or game wins per bot
    wins: [u32; 2],
    /// Round points won (rounds mode) or winning game totals per bot
    points: [u64; 2],
    knocks: u32,
    undercuts: u32,
    gins: u32,
    big_gins: u32,
    dead: u32,
    /// Trials merged in — the paired test's sample size
    trials: u32,
    /// Sum of per-trial win differences, bot 1 minus bot 2, and of their
    /// squares: the sufficient statistics of the paired significance test
    d_sum: i64,
    d2_sum: u64,
}

impl Outcome {
    fn merge(mut self, other: Self) -> Self {
        for bot in 0..2 {
            self.wins[bot] += other.wins[bot];
            self.points[bot] += other.points[bot];
        }
        self.knocks += other.knocks;
        self.undercuts += other.undercuts;
        self.gins += other.gins;
        self.big_gins += other.big_gins;
        self.dead += other.dead;
        self.trials += other.trials;
        self.d_sum += other.d_sum;
        self.d2_sum += other.d2_sum;
        self
    }

    /// Record a round result; `bot_of_seat` maps each seat to a bot index.
    fn record_round(&mut self, result: RoundResult, rules: &Rules, bot_of_seat: [usize; 2]) {
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

/// Play a whole game alternating the dealer every hand — the EAAI
/// challenge's protocol — instead of [`Game`]'s winner-deals-next.  The
/// `Game` serves purely as the scoreboard.
fn alternate_dealer_game(
    rules: Rules,
    first_dealer: Player,
    strategies: [&mut dyn Strategy; 2],
    rng: &mut StdRng,
) -> Result<FinalScore> {
    let [one, two] = strategies;
    let mut game = Game::new(rules, first_dealer);
    let mut dealer = first_dealer;
    while !game.is_over() {
        let scores = [game.score(Player::One), game.score(Player::Two)];
        let mut table = Table::new(Round::deal(rules, dealer, rng)).scores(scores);
        game.record(table.play([&mut *one, &mut *two])?)?;
        // The deal alternates every hand, dead hands included.
        dealer = dealer.opponent();
    }
    Ok(game.final_score().expect("a game that is over settles"))
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
    let dealt = (!config.games)
        .then(|| Table::deal(config.rules, dealer, &mut StdRng::seed_from_u64(trial_seed)));

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
            // Identically seeded deal streams pair the orientations: round
            // k of either game is dealt from the same shuffle (exactly
            // mirrored under --alternate-dealer, whose dealer sequence
            // cannot diverge between the orientations).
            let mut rng = StdRng::seed_from_u64(trial_seed);
            let score = if config.alternate_dealer {
                alternate_dealer_game(config.rules, dealer, seats, &mut rng)?
            } else {
                let mut game = Game::new(config.rules, dealer);
                play_game(&mut game, seats, &mut rng)?
            };
            let bot = bot_of_seat[score.winner as usize];
            outcome.wins[bot] += 1;
            outcome.points[bot] += u64::from(score.totals[score.winner as usize]);
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
    Ok(outcome)
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

/// The two-sided normal p-value of a z statistic
fn p_value(z: f64) -> f64 {
    // erfc via Abramowitz & Stegun 7.1.26, |error| < 1.5e-7 — plenty for a
    // significance readout.
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    poly * (-x * x).exp()
}

/// Print the head-to-head significance line: the paired z over per-trial
/// win differences, or the sign test when trials are unpaired
fn significance(outcome: &Outcome, config: &Config, denominator: u32) {
    let diff = 100.0 * (f64::from(outcome.wins[0]) - f64::from(outcome.wins[1]))
        / f64::from(denominator.max(1));
    let header = format!("p1={} vs p2={}: diff {diff:+.1}pp", config.p1, config.p2);
    if config.paired {
        let n = f64::from(outcome.trials);
        let mean = outcome.d_sum as f64 / n;
        let var = ((outcome.d2_sum as f64) - n * mean * mean) / (n - 1.0).max(1.0);
        if var <= 0.0 {
            // Every trial tied (or agreed exactly): no discordance to test.
            println!("{header}, no discordant trials");
        } else {
            let z = mean / (var / n).sqrt();
            println!("{header}, paired z = {z:.2}, p = {:.3}", p_value(z));
        }
    } else {
        let decisive = outcome.wins[0] + outcome.wins[1];
        if decisive == 0 {
            println!("{header}, no decisive trials");
        } else {
            let z = (f64::from(outcome.wins[0]) - f64::from(outcome.wins[1]))
                / f64::from(decisive).sqrt();
            println!("{header}, sign z = {z:.2}, p = {:.3}", p_value(z));
        }
    }
}

fn main() -> Result<()> {
    let config = parse_args()?;
    let seed = config.seed.unwrap_or_else(|| rand::rng().random());
    let start = Instant::now();

    let outcome = (0..config.count)
        .into_par_iter()
        .map(|index| trial(&config, seed, index))
        .try_reduce(Outcome::default, |a, b| Ok(a.merge(b)))?;

    let elapsed = start.elapsed();
    let per_trial = if config.paired { 2 } else { 1 };
    let total = config.count * per_trial;
    let unit = if config.games { "game" } else { "round" };
    let rate = f64::from(total) / elapsed.as_secs_f64();
    if config.paired {
        println!(
            "{} mirrored pairs ({total} {unit}s) in {elapsed:.2?} ({rate:.1} {unit}s/s), seed {seed}",
            config.count,
        );
    } else {
        println!("{total} {unit}s in {elapsed:.2?} ({rate:.1} {unit}s/s), seed {seed}");
    }

    let names = [format!("p1={}", config.p1), format!("p2={}", config.p2)];
    if config.games {
        for (bot, name) in names.iter().enumerate() {
            let (lo, hi) = wilson(outcome.wins[bot], total);
            println!(
                "{name}: {} game wins / {total} ({:.1}%, 95% CI {:.1}%\u{2013}{:.1}%), {} winning points",
                outcome.wins[bot],
                100.0 * f64::from(outcome.wins[bot]) / f64::from(total.max(1)),
                100.0 * lo,
                100.0 * hi,
                outcome.points[bot],
            );
        }
        significance(&outcome, &config, total);
    } else {
        let decisive = outcome.wins[0] + outcome.wins[1];
        for (bot, name) in names.iter().enumerate() {
            let (lo, hi) = wilson(outcome.wins[bot], decisive);
            println!(
                "{name}: {} wins / {decisive} decisive ({:.1}%, 95% CI {:.1}%\u{2013}{:.1}%), {:.2} points/round",
                outcome.wins[bot],
                100.0 * f64::from(outcome.wins[bot]) / f64::from(decisive.max(1)),
                100.0 * lo,
                100.0 * hi,
                outcome.points[bot] as f64 / f64::from(total.max(1)),
            );
        }
        println!(
            "results: {} knocks, {} undercuts, {} gins, {} big gins, {} dead hands",
            outcome.knocks, outcome.undercuts, outcome.gins, outcome.big_gins, outcome.dead,
        );
        significance(&outcome, &config, decisive);
    }
    Ok(())
}
