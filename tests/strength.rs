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
//! Three floors, each guarding a different claim, all set well below the
//! corrected fixed-seed fixture: a tripwire catches accidents, while
//! `examples/arena.rs` does measurement.
//!
//! - `monte_carlo_beats_the_heuristic` — head-to-head round play.  The
//!   default `HeuristicBot` is tuned for whole-game play and concedes
//!   single rounds; the current fixture realizes 538/988 decisive wins
//!   (54.45%).  The 52.5% bar leaves 1.95 points of headroom, while an even
//!   bot slips through less than 6% of the time.  Minutes.
//! - `greedy_beats_eaai_baseline_on_games` and
//!   `monte_carlo_beats_eaai_baseline_on_games` — whole-game play against
//!   the EAAI-2021 baseline under the challenge protocol, the number this
//!   crate is aimed at.  The round tripwire above cannot see it: both
//!   scores stay 0–0 inside a single round, where the game-win value
//!   function is locally linear and therefore nearly invisible.  These
//!   corrected fixtures realize exactly 2386/4000 (59.65%) and 679/1000
//!   (67.9%).  Their conservative 55% and 60.9% floors leave 4.65 and 7
//!   percentage points of regression headroom.  Pair-cluster intervals are
//!   printed when these ignored tests run.  Seconds and ~4 minutes.

#![cfg(feature = "rand")]

use gin_rummy::{Game, Player, Round, Rules};
use gin_rummy_engine::{
    DealerRotation, EaaiSimpleBot, HeuristicBot, MonteCarloBot, Strategy, Table, eaai_rules,
};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rayon::prelude::*;

/// Golden-ratio odd constant, the arena's per-trial seed mixer
const MIX: u64 = 0x9E37_79B9_7F4A_7C15;
const GREEDY_GAME_FLOOR: f64 = 0.55;
const MONTE_CARLO_GAME_FLOOR: f64 = 0.609;

/// Sufficient statistics for a mirrored-pair game-win rate.
///
/// A pair is one independent cluster and contributes zero, one, or two
/// wins.  Keeping the sum of squared pair wins preserves the within-pair
/// dependence that an individual-game binomial interval would discard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PairedRate {
    pairs: u32,
    wins: u32,
    wins_sq: u32,
}

impl PairedRate {
    fn one(pair_wins: u32) -> Self {
        debug_assert!(pair_wins <= 2);
        Self {
            pairs: 1,
            wins: pair_wins,
            wins_sq: pair_wins * pair_wins,
        }
    }

    const fn merge(self, other: Self) -> Self {
        Self {
            pairs: self.pairs + other.pairs,
            wins: self.wins + other.wins,
            wins_sq: self.wins_sq + other.wins_sq,
        }
    }

    fn rate(self) -> f64 {
        f64::from(self.wins) / f64::from(2 * self.pairs)
    }

    /// A normal 95% interval with mirrored pairs as clusters.
    fn cluster_ci95(self) -> (f64, f64) {
        assert!(
            self.pairs > 1,
            "a cluster interval needs at least two pairs"
        );
        let clusters = f64::from(self.pairs);
        let wins = f64::from(self.wins);
        let estimate = self.rate();
        let denominator = 2.0 * clusters;
        let residual_ss =
            f64::from(self.wins_sq) - 4.0 * estimate * wins + 4.0 * estimate * estimate * clusters;
        let variance =
            clusters * residual_ss.max(0.0) / ((clusters - 1.0) * denominator * denominator);
        let half = 1.96 * variance.sqrt();
        ((estimate - half).max(0.0), (estimate + half).min(1.0))
    }
}

/// Play a whole game under the EAAI challenge's dealer protocol.
///
/// A scored round flips the starting role; a dead round is cancelled and
/// redealt by the same role.  This differs from [`Game`]'s
/// winner-deals-next bookkeeping, so the loop owns the dealer explicitly.
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
        let mut table = Table::new(Round::deal(rules, dealer, rng))
            .scores(scores)
            .dealer_rotation(DealerRotation::AlternateAfterScoredRound);
        let result = table
            .play([&mut *one, &mut *two])
            .expect("bots play legally");
        let scored = result.winner().is_some();
        game.record(result)
            .expect("a round of the game in progress");
        if scored {
            dealer = dealer.opponent();
        }
    }
    game.winner().expect("a game that is over has a winner")
}

/// Play trial `index` as a mirrored pair against the baseline, returning
/// how many of the two games the challenger won
///
/// Both orientations start from the same seeded deal stream from opposite
/// seats.  Later dealer roles can diverge when only one orientation has a
/// dead round, so this is a common-random-number pair rather than an exact
/// whole-game replay.
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
    eprintln!(
        "Monte Carlo round fixture: {}/{decisive} decisive rounds ({:.2}%)",
        wins[0],
        100.0 * rate,
    );
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
    let stats = (0..PAIRS)
        .map(|index| paired_trial(11, index, |_| Box::new(HeuristicBot::new())))
        .map(PairedRate::one)
        .fold(PairedRate::default(), PairedRate::merge);

    let games = 2 * stats.pairs;
    let rate = stats.rate();
    let (low, high) = stats.cluster_ci95();
    eprintln!(
        "corrected greedy fixture: {}/{games} games ({:.2}%, pair-cluster 95% CI {:.2}%–{:.2}%)",
        stats.wins,
        100.0 * rate,
        100.0 * low,
        100.0 * high,
    );
    // The corrected fixed fixture is 2386/4000 (59.65%).  The tripwire
    // leaves 4.65 percentage points of headroom rather than treating the
    // two orientations of each pair as independent observations.
    assert!(
        rate > GREEDY_GAME_FLOOR,
        "the heuristic won only {}/{games} games ({:.1}%, pair-cluster 95% CI {:.1}%–{:.1}%; floor {:.1}%)",
        stats.wins,
        100.0 * rate,
        100.0 * low,
        100.0 * high,
        100.0 * GREEDY_GAME_FLOOR,
    );
}

#[test]
#[ignore = "statistical, minutes long; run with --release -- --ignored"]
fn monte_carlo_beats_eaai_baseline_on_games() {
    const PAIRS: u32 = 500;
    let stats = (0..PAIRS)
        .into_par_iter()
        .map(|index| {
            paired_trial(13, index, |seed| {
                Box::new(MonteCarloBot::new(StdRng::seed_from_u64(seed)))
            })
        })
        .map(PairedRate::one)
        .reduce(PairedRate::default, PairedRate::merge);

    let games = 2 * stats.pairs;
    let rate = stats.rate();
    let (low, high) = stats.cluster_ci95();
    eprintln!(
        "corrected Monte Carlo fixture: {}/{games} games ({:.2}%, pair-cluster 95% CI {:.2}%–{:.2}%)",
        stats.wins,
        100.0 * rate,
        100.0 * low,
        100.0 * high,
    );
    // The corrected fixed fixture is 679/1000 (67.9%).  The smaller
    // fixture keeps a wider seven-point cushion between its observed rate
    // and this regression floor.
    assert!(
        rate > MONTE_CARLO_GAME_FLOOR,
        "Monte Carlo won only {}/{games} games ({:.1}%, pair-cluster 95% CI {:.1}%–{:.1}%; floor {:.1}%)",
        stats.wins,
        100.0 * rate,
        100.0 * low,
        100.0 * high,
        100.0 * MONTE_CARLO_GAME_FLOOR,
    );
}

#[test]
fn paired_rate_interval_uses_pairs_as_clusters() {
    let stats = [2, 0, 1, 1]
        .into_iter()
        .map(PairedRate::one)
        .fold(PairedRate::default(), PairedRate::merge);
    assert_eq!(stats.pairs, 4);
    assert_eq!(stats.wins, 4);
    assert_eq!(stats.wins_sq, 6);
    assert_eq!(stats.rate(), 0.5);
    let (low, high) = stats.cluster_ci95();
    assert!(low < 0.5 && high > 0.5);
}
