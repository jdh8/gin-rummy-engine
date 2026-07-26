//! Statistical strength tripwires, ignored by default
//!
//! ```console
//! cargo test --release --test strength -- --ignored
//! ```
//!
//! Debug builds are far too slow for Monte Carlo rollouts at this scale;
//! always run these in release mode.  Note that the command above leaves
//! the `parallel` feature off, which is what keeps the game tripwire's
//! trial-level fan-out from competing with Monte Carlo's in-decision
//! rayon pool for the same threads.
//!
//! Three floors, each guarding a different claim, all set far below the
//! measured rate: a tripwire catches accidents, while `examples/arena.rs`
//! does measurement.
//!
//! - `monte_carlo_beats_the_heuristic` — head-to-head round play.  The
//!   default `HeuristicBot` is tuned for whole-game play and concedes
//!   single rounds, so `mc:128` takes about 65% of them; the 52.5% bar
//!   over 1000 rounds then passes with overwhelming probability, while an
//!   even bot slips through less than 6% of the time.  Minutes.
//! - `greedy_beats_eaai_baseline_on_games` and
//!   `monte_carlo_beats_eaai_baseline_on_games` — whole-game play against
//!   the EAAI-2021 baseline under the challenge protocol, the number this
//!   crate is aimed at.  The round tripwire above cannot see it: both
//!   scores stay 0–0 inside a single round, where the game-win value
//!   function is locally linear and therefore nearly invisible.  These
//!   fixtures realize 59.6% and 59.5%, agreeing with the README's pinned
//!   measurements, and their floors sit ~4σ below that.  Seconds and ~4
//!   minutes.

#![cfg(feature = "rand")]

use gin_rummy::{Game, Player, Round, Rules};
use gin_rummy_engine::{EaaiSimpleBot, HeuristicBot, MonteCarloBot, Strategy, Table};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rayon::prelude::*;

/// Golden-ratio odd constant, the arena's per-trial seed mixer
const MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// The EAAI-2021 challenge's round conditions: modern bonuses, no big gin
fn eaai_rules() -> Rules {
    let mut rules = Rules::new();
    rules.big_gin_bonus = None;
    rules
}

/// Play a whole game alternating the dealer every hand — the EAAI
/// challenge's protocol — instead of [`Game`]'s winner-deals-next,
/// returning the winner.
///
/// Duplicated from `examples/arena.rs`, which cannot export code to a
/// test; the alternative is a public API grown for one caller.
fn alternate_dealer_game(
    rules: Rules,
    first_dealer: Player,
    strategies: [&mut dyn Strategy; 2],
    rng: &mut StdRng,
) -> Player {
    let [one, two] = strategies;
    let mut game = Game::new(rules, first_dealer);
    let mut dealer = first_dealer;
    while !game.is_over() {
        let scores = [game.score(Player::One), game.score(Player::Two)];
        let mut table = Table::new(Round::deal(rules, dealer, rng)).scores(scores);
        let result = table
            .play([&mut *one, &mut *two])
            .expect("bots play legally");
        game.record(result)
            .expect("a round of the game in progress");
        // The deal alternates every hand, dead hands included.
        dealer = dealer.opponent();
    }
    game.final_score()
        .expect("a game that is over settles")
        .winner
}

/// Play trial `index` as a mirrored pair against the baseline, returning
/// how many of the two games the challenger won
///
/// Both orientations replay the same seeded deal stream from opposite
/// seats, so deal luck cancels; seeding from the index alone keeps the
/// count identical however the trials schedule.
fn paired_trial(
    seed: u64,
    index: u32,
    mut challenger: impl FnMut(u64) -> Box<dyn Strategy>,
) -> u32 {
    let trial_seed = seed ^ u64::from(index).wrapping_mul(MIX);
    let dealer = if index % 4 < 2 {
        Player::One
    } else {
        Player::Two
    };
    let mut wins = 0;
    for swapped in [false, true] {
        let mut ours = challenger(trial_seed);
        let mut theirs = EaaiSimpleBot::new(StdRng::seed_from_u64(trial_seed ^ MIX));
        let seats: [&mut dyn Strategy; 2] = if swapped {
            [&mut theirs, &mut *ours]
        } else {
            [&mut *ours, &mut theirs]
        };
        let mut rng = StdRng::seed_from_u64(trial_seed);
        let winner = alternate_dealer_game(eaai_rules(), dealer, seats, &mut rng);
        wins += u32::from((winner == Player::One) != swapped);
    }
    wins
}

#[test]
#[ignore = "statistical, minutes long; run with --release -- --ignored"]
fn monte_carlo_beats_the_heuristic() {
    const ROUNDS: u32 = 1000;
    let mut rng = StdRng::seed_from_u64(2026);
    let mut greedy = HeuristicBot::new();
    let mut mc = MonteCarloBot::new(StdRng::seed_from_u64(7)).samples(128);
    let mut wins = [0u32; 2];

    for index in 0..ROUNDS {
        let swapped = index % 2 == 1;
        let dealer = if index % 4 < 2 {
            Player::One
        } else {
            Player::Two
        };
        let mut table = Table::deal(Rules::default(), dealer, &mut rng);
        let seats: [&mut dyn Strategy; 2] = if swapped {
            [&mut greedy, &mut mc]
        } else {
            [&mut mc, &mut greedy]
        };
        let result = table.play(seats).expect("bots play legally");
        if let Some(winner) = result.winner() {
            let mc_won = (winner == Player::One) != swapped;
            wins[usize::from(!mc_won)] += 1;
        }
    }

    let decisive = wins[0] + wins[1];
    let rate = f64::from(wins[0]) / f64::from(decisive);
    assert!(
        rate > 0.525,
        "Monte Carlo won only {}/{decisive} decisive rounds ({:.1}%)",
        wins[0],
        100.0 * rate,
    );
}

#[test]
#[ignore = "statistical; run with --release -- --ignored"]
fn greedy_beats_eaai_baseline_on_games() {
    const PAIRS: u32 = 2000;
    let wins: u32 = (0..PAIRS)
        .map(|index| paired_trial(11, index, |_| Box::new(HeuristicBot::new())))
        .sum();

    let games = 2 * PAIRS;
    let rate = f64::from(wins) / f64::from(games);
    // This fixture realizes 59.6%, the arena's pinned figure; at 4000
    // games the sampling sd is 0.78pp, so 56% sits 4.6σ below it.
    assert!(
        rate > 0.56,
        "the heuristic won only {wins}/{games} games ({:.1}%)",
        100.0 * rate,
    );
}

#[test]
#[ignore = "statistical, minutes long; run with --release -- --ignored"]
fn monte_carlo_beats_eaai_baseline_on_games() {
    const PAIRS: u32 = 500;
    let wins: u32 = (0..PAIRS)
        .into_par_iter()
        .map(|index| {
            paired_trial(13, index, |seed| {
                Box::new(MonteCarloBot::new(StdRng::seed_from_u64(seed)))
            })
        })
        .sum();

    let games = 2 * PAIRS;
    let rate = f64::from(wins) / f64::from(games);
    // This fixture realizes 59.5%, the arena's pinned figure for the
    // default bot; at 1000 games the sd is 1.55pp, so 53% sits 4.2σ down.
    assert!(
        rate > 0.53,
        "Monte Carlo won only {wins}/{games} games ({:.1}%)",
        100.0 * rate,
    );
}
