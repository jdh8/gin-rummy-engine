//! How long a bot takes to decide
//!
//! ```console
//! cargo bench
//! ```

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use gin_rummy::{Hand, Phase, Player, Round, Rules};
use gin_rummy_engine::{
    DrawAction, HeuristicBot, Layoff, MonteCarloBot, Strategy, Table, TurnAction, UpcardAction,
    View, play_round,
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

/// Greedy self-play from the stride-3 deal, both seats knocking only on
/// gin: `T` takes the upcard or discard, `S` draws from the stock, and a
/// card is the shed
///
/// Replaying the recording, rather than the bot, keeps the benchmarked
/// positions fixed when the greedy core changes.  Re-record only after a
/// deliberate mechanics change makes a move illegal.
const SCRIPT: &str = "T K♠ S K♣ S T♣ S J♠ S 9♣ T T♠ S Q♥ S J♥ S 9♦ T Q♦ S 8♠ \
    S K♦ S T♦ S 7♦ T 8♥ S 7♣ T 6♣ S 6♦ S J♣ S 8♣ S 6♥ S 5♥ S Q♠ S 5♠ S";

/// Plays back recorded decisions in order
struct Script(std::vec::IntoIter<&'static str>);

impl Script {
    fn next(&mut self) -> &'static str {
        self.0.next().expect("the script reaches the position")
    }
}

impl Strategy for Script {
    fn offer_upcard(&mut self, _: &View<'_>) -> UpcardAction {
        match self.next() {
            "T" => UpcardAction::Take,
            _ => UpcardAction::Pass,
        }
    }
    fn choose_draw(&mut self, _: &View<'_>) -> DrawAction {
        match self.next() {
            "T" => DrawAction::TakeDiscard,
            _ => DrawAction::Stock,
        }
    }
    fn play_turn(&mut self, _: &View<'_>) -> TurnAction {
        TurnAction::Discard(self.next().parse().expect("a recorded card"))
    }
    fn choose_layoff(&mut self, _: &View<'_>) -> Option<Layoff> {
        unreachable!("the script ends before any knock")
    }
}

/// The stride-3 deal after the first `moves` recorded decisions, checked
/// against the stock size and hand the recording paused on
fn position(moves: usize, stock: usize, hand: &str) -> (Table, Player) {
    let moves: Vec<_> = SCRIPT.split_whitespace().take(moves).collect();
    let mut script = Script(moves.into_iter());
    let mut table = Table::new(fixed_deal(3));
    while !script.0.as_slice().is_empty() {
        table
            .step(&mut script)
            .expect("the recorded decisions are legal");
    }
    let seat = table.turn().expect("the round outlasts the script");
    let view = table.view(seat);
    assert_eq!(
        (view.stock_len(), view.hand()),
        (stock, hand.parse().expect("a recorded hand")),
        "the replay reaches the recorded position",
    );
    (table, seat)
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
    // adds knock candidates.
    let cases = [
        ("draw", position(28, 20, "46.3.2368.A27")),
        ("late discard", position(49, 11, "457.37.23.A267")),
        ("knock", position(47, 12, "A239.A49.9.459")),
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
