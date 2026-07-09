//! How long a bot takes to decide
//!
//! ```console
//! cargo bench
//! ```

use criterion::{Criterion, criterion_group, criterion_main};
use gin_rummy::{Hand, Player, Round, Rules};
use gin_rummy_engine::{
    DrawAction, HeuristicBot, Layoff, MonteCarloBot, Strategy, Table, TurnAction, UpcardAction,
    View, play_round,
};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use std::hint::black_box;

fn fixed_deal() -> Round {
    let deck: Vec<_> = Hand::ALL.iter().collect();
    let hands = [
        deck.iter().step_by(2).take(10).copied().collect::<Hand>(),
        deck.iter().skip(1).step_by(2).take(10).copied().collect(),
    ];
    Round::from_deal(
        Rules::default(),
        Player::One,
        hands,
        deck[20],
        deck[21..].to_vec(),
    )
    .expect("a partitioned deck")
}

/// Takes the upcard, leaving the non-dealer an 11-card turn to decide
struct Taker;

impl Strategy for Taker {
    fn offer_upcard(&mut self, _: &View<'_>) -> UpcardAction {
        UpcardAction::Take
    }
    fn choose_draw(&mut self, _: &View<'_>) -> DrawAction {
        DrawAction::TakeDiscard
    }
    fn play_turn(&mut self, _: &View<'_>) -> TurnAction {
        unreachable!("the benches stop at the discard decision")
    }
    fn choose_layoff(&mut self, _: &View<'_>) -> Option<Layoff> {
        None
    }
}

/// A table paused on the non-dealer's 11-card discard decision
fn discard_position() -> Table {
    let mut table = Table::new(fixed_deal());
    table.step(&mut Taker).expect("taking the upcard is legal");
    table
}

fn decisions(c: &mut Criterion) {
    let table = discard_position();

    c.bench_function("heuristic turn", |b| {
        let mut bot = HeuristicBot::new();
        b.iter(|| black_box(bot.play_turn(&table.view(Player::Two))));
    });

    c.bench_function("greedy round", |b| {
        b.iter(|| {
            play_round(
                black_box(fixed_deal()),
                [&mut HeuristicBot::new(), &mut HeuristicBot::new()],
            )
            .expect("legal play")
        });
    });

    for samples in [16, 64, 128] {
        c.bench_function(&format!("monte carlo turn, {samples} samples"), |b| {
            let mut bot = MonteCarloBot::new(StdRng::seed_from_u64(1)).samples(samples);
            b.iter(|| black_box(bot.play_turn(&table.view(Player::Two))));
        });
    }

    // An Expert-sized decision is slow enough that criterion's default 100
    // measurements would take minutes; 10 keeps the arm honest and quick.
    let mut group = c.benchmark_group("expert");
    group.sample_size(10);
    group.bench_function("monte carlo turn, 1024 samples", |b| {
        let mut bot = MonteCarloBot::new(StdRng::seed_from_u64(1)).samples(1024);
        b.iter(|| black_box(bot.play_turn(&table.view(Player::Two))));
    });
    group.finish();
}

criterion_group!(benches, decisions);
criterion_main!(benches);
