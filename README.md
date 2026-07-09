# Gin Rummy Engine

[![Crates.io](https://img.shields.io/crates/v/gin-rummy-engine)](https://crates.io/crates/gin-rummy-engine)
[![Docs.rs](https://docs.rs/gin-rummy-engine/badge.svg)](https://docs.rs/gin-rummy-engine)
[![Build Status](https://github.com/jdh8/gin-rummy-engine/actions/workflows/rust.yml/badge.svg)](https://github.com/jdh8/gin-rummy-engine)

Bots and strategy tooling for [gin rummy], built on the [gin-rummy] mechanics
crate.  Where gin-rummy answers *"what moves are legal?"*, this crate answers
*"which move should I make?"*

The design triangle:

- [`Strategy`]: a decision procedure for one seat — take or pass the upcard,
  where to draw, what to shed, whether to knock, what to lay off.
- [`View`]: the information a seat may legally see.  The underlying [`Round`]
  exposes both hands and the stock order; strategies never touch it.  A
  `View` shows only the seat's own hand, the discard pile, the stock *count*,
  and what the opponent has revealed (cards taken from the pile, discards,
  declined upcards).
- [`Table`]: the driver.  It owns the `Round`, tracks each seat's knowledge,
  asks strategies for decisions, and applies them — so information hygiene
  holds by construction.

## Bots

- [`HeuristicBot`]: deterministic and fast.  Draws from the pile only when
  that strictly lowers deadwood, sheds the least useful card weighted by how
  dangerous it is to the opponent, knocks by a configurable threshold, and
  lays off greedily but never breaks its own melds.
- [`MonteCarloBot`] (feature `rand`): determinized Monte Carlo.  At each
  decision it samples hidden worlds consistent with the `View` — opponent
  hands containing every known card, random stock orders over the unseen
  cards — rolls each out with the greedy policy, and picks the action with
  the best expected score.

## Quick start

A bot-vs-bot round needs no features:

```rust
use gin_rummy::{Hand, Player, Round, Rules};
use gin_rummy_engine::{HeuristicBot, play_round};

let hands: [Hand; 2] = ["A23.456.789.T".parse()?, "TJQK.A23.456.".parse()?];
# let rest: Vec<_> = (Hand::ALL - (hands[0] | hands[1])).iter().collect();
let (upcard, stock) = (rest[0], rest[1..].to_vec());  // the other 32 cards
let round = Round::from_deal(Rules::default(), Player::One, hands, upcard, stock)?;
let result = play_round(round, [&mut HeuristicBot::new(), &mut HeuristicBot::new()])?;
println!("{result:?}");
# Ok::<(), Box<dyn std::error::Error>>(())
```

With the (default) `rand` feature, deal and settle whole games:

```rust
# #[cfg(feature = "rand")]
# fn main() -> Result<(), gin_rummy_engine::EngineError> {
use gin_rummy::{Game, Player, Rules};
use gin_rummy_engine::{HeuristicBot, MonteCarloBot, play_game};

let mut rules = Rules::default();
# rules.game_target = 1; // a single round settles this demo game
let mut game = Game::new(rules, Player::One);
let mut greedy = HeuristicBot::new();
let mut mc = MonteCarloBot::new(rand::rng()).samples(8);
let score = play_game(&mut game, [&mut greedy, &mut mc], &mut rand::rng())?;
println!("{} wins {} : {}", score.winner, score.totals[0], score.totals[1]);
# Ok(())
# }
# #[cfg(not(feature = "rand"))]
# fn main() {}
```

Writing your own bot is implementing [`Strategy`]'s four decisions against a
[`View`]; the driver handles all bookkeeping.

## Feature flags

- `rand` (default): the Monte Carlo bot, `Table::deal`, `play_game`, and the
  examples.  Disable it for a dependency-free heuristic-only build.
- `parallel`: Monte Carlo rollouts across the CPU cores via rayon.
  Decisions are bit-identical to the serial build, each just arrives
  faster; worthwhile at high sample counts.  Off by default.

## Examples

- `play`: play against a bot in the terminal —
  `cargo run --example play` (`--bot mc`, `--rules classic`, …)
- `arena`: bot-vs-bot tournaments with win-rate statistics —
  `cargo run --release --example arena -- --rounds 1000 --p1 greedy --p2 mc:64`

[gin rummy]: https://www.pagat.com/rummy/ginrummy.html
[gin-rummy]: https://crates.io/crates/gin-rummy
[`Strategy`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/trait.Strategy.html
[`View`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.View.html
[`Table`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.Table.html
[`HeuristicBot`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.HeuristicBot.html
[`MonteCarloBot`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.MonteCarloBot.html
[`Round`]: https://docs.rs/gin-rummy/latest/gin_rummy/round/struct.Round.html
