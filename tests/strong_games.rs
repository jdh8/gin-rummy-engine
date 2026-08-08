//! Whole-game legality, reset, termination, and replay checks for adapters.

#![cfg(feature = "rand")]

#[allow(dead_code)]
#[path = "../examples/support/strong/mod.rs"]
mod strong;

use gin_rummy::{Game, Player, Round, RoundResult};
use gin_rummy_engine::{DealerRotation, HeuristicBot, Strategy, Table, eaai_rules};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use strong::{GoldPaperBot, MarjjV5Surrogate};

fn replay_game(seed: u64, opponent: Box<dyn Strategy>) -> Vec<RoundResult> {
    let rules = eaai_rules();
    let mut rng = StdRng::seed_from_u64(seed);
    let mut game = Game::new(rules, Player::One);
    let mut dealer = Player::One;
    let mut candidate = HeuristicBot::new();
    let mut opponent = opponent;
    let mut results = Vec::new();
    while !game.is_over() {
        assert!(results.len() < 1_000, "a benchmark game must terminate");
        let scores = [game.score(Player::One), game.score(Player::Two)];
        let mut table = Table::new(Round::deal(rules, dealer, &mut rng))
            .scores(scores)
            .dealer_rotation(DealerRotation::AlternateAfterScoredRound);
        let result = table
            .play([&mut candidate, &mut *opponent])
            .expect("the adapter always returns legal actions");
        game.record(result).expect("a host result records");
        if result.winner().is_some() {
            dealer = dealer.opponent();
        }
        results.push(result);
    }
    assert!(game.winner().is_some(), "the raw score reaches target");
    results
}

#[test]
fn gold_games_terminate_legally_and_replay() {
    for seed in [7, 8] {
        let first = replay_game(seed, Box::new(GoldPaperBot::new()));
        let second = replay_game(seed, Box::new(GoldPaperBot::new()));
        assert_eq!(first, second);
    }
}

#[test]
fn marjj_games_reset_terminate_legally_and_replay() {
    for seed in [7, 8] {
        let make = || {
            Box::new(MarjjV5Surrogate::new(StdRng::seed_from_u64(
                seed ^ 0x004d_4152_4a4a,
            )))
        };
        let first = replay_game(seed, make());
        let second = replay_game(seed, make());
        assert_eq!(first, second);
    }
}
