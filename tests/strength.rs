//! Statistical strength tripwire, ignored by default
//!
//! ```console
//! cargo test --release --test strength -- --ignored
//! ```
//!
//! Debug builds are far too slow for Monte Carlo rollouts at this scale;
//! always run it in release mode.  With the bot's true win rate around
//! 56%, the 52.5% bar over 1000 rounds passes with better than 99%
//! probability, while an even bot slips through less than 6% of the time.

#![cfg(feature = "rand")]

use gin_rummy::{Player, Rules};
use gin_rummy_engine::{HeuristicBot, MonteCarloBot, Strategy, Table};
use rand::SeedableRng as _;
use rand::rngs::StdRng;

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
