//! How long a bot takes to decide
//!
//! ```console
//! cargo bench
//! ```

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use gin_rummy::{Hand, Phase, Player, Round, Rules, deadwood};
use gin_rummy_engine::{
    DrawAction, HeuristicBot, HeuristicConfig, Layoff, MonteCarloBot, Strategy, Table, TurnAction,
    UpcardAction, View, play_round,
};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use std::hint::black_box;

/// The sorted deck, reordered by `stride` (coprime to 52), dealt
/// round-robin
fn fixed_deal(stride: usize) -> Round {
    let sorted: Vec<_> = Hand::ALL.iter().collect();
    let deck: Vec<_> = (0..52).map(|i| sorted[i * stride % 52]).collect();
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
    let mut table = Table::new(fixed_deal(1));
    table.step(&mut Taker).expect("taking the upcard is legal");
    table
}

/// Whether the 11-card hand in `view` can knock without going gin, the
/// only discard where the Monte Carlo bot also weighs a knock
fn can_knock(view: &View<'_>) -> bool {
    let hand = view.hand();
    hand.iter()
        .map(|card| deadwood(hand - card.into()))
        .any(|dw| (1..=view.knock_limit()).contains(&dw))
}

/// Greedy self-play from the stride-3 deal, paused on the first decision
/// where `stop` holds for the seat to act
///
/// Both seats knock only on gin, so the round runs past every knockable
/// hand; on this deal it runs to a dead hand, through all three cases.
fn position(stop: impl Fn(&View<'_>) -> bool) -> (Table, Player) {
    let mut config = HeuristicConfig::default();
    config.knock_threshold = 0;
    let mut bot = HeuristicBot::with_config(config);
    let mut table = Table::new(fixed_deal(3));
    while let Some(seat) = table.turn() {
        if stop(&table.view(seat)) {
            return (table, seat);
        }
        table.step(&mut bot).expect("greedy play is legal");
    }
    panic!("the stride-3 deal reaches the benchmarked position");
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
                black_box(fixed_deal(1)),
                [&mut HeuristicBot::new(), &mut HeuristicBot::new()],
            )
            .expect("legal play")
        });
    });

    for samples in [16, 64, 128, 256, 512] {
        c.bench_function(&format!("monte carlo turn, {samples} samples"), |b| {
            let mut bot = MonteCarloBot::new(StdRng::seed_from_u64(1)).samples(samples);
            b.iter(|| black_box(bot.play_turn(&table.view(Player::Two))));
        });
    }

    // Later positions cost differently: a draw scores the take against the
    // stock, a late discard rolls out fewer turns, and a knockable hand
    // adds knock candidates.  On the stride-3 deal these land at stock 20,
    // 11, and 12.
    let cases = [
        (
            "draw",
            position(|v| v.phase() == Phase::Draw && v.can_take_discard() && v.stock_len() <= 20),
        ),
        (
            "late discard",
            position(|v| v.phase() == Phase::Discard && v.stock_len() <= 12 && !can_knock(v)),
        ),
        (
            "knock",
            position(|v| v.phase() == Phase::Discard && can_knock(v)),
        ),
    ];
    for (name, (table, seat)) in &cases {
        for samples in [128, 512] {
            c.bench_function(&format!("monte carlo {name}, {samples} samples"), |b| {
                // A fresh bot per decision: a reused one would see the draw
                // position recur and short-circuit to the stock.
                b.iter_batched(
                    || MonteCarloBot::new(StdRng::seed_from_u64(1)).samples(samples),
                    |mut bot| {
                        let view = table.view(*seat);
                        if view.phase() == Phase::Draw {
                            black_box(bot.choose_draw(&view));
                        } else {
                            black_box(bot.play_turn(&view));
                        }
                    },
                    BatchSize::SmallInput,
                );
            });
        }
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
