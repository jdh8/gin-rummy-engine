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
  the best chance of winning the *game*.  A round outcome is priced through
  a solved win-probability function of both game scores, so the search
  banks a lead and presses a deficit instead of valuing every round point
  alike.  [`McConfig`] exposes the search's levers (the rollout knock
  thresholds per seat, the modeled opponent's draw rule, the significance
  gate, the candidate cap, the sampled opponent's strength, and the value
  function itself).
- [`EaaiSimpleBot`] (feature `rand`): a port of `SimpleGinRummyPlayer`, the
  baseline every entry of the EAAI-2021 Gin Rummy AI challenge was measured
  against.  Deliberately weak and knob-free — it exists so that win rates
  against it are comparable across engines and papers.

## Benchmarks

[`EaaiSimpleBot`] is the yardstick: arena runs under `--rules eaai` (the
challenge's round conditions) with `--alternate-dealer` (the challenge's
dealer protocol), every trial a mirrored pair — both bots play the same
deals from both seats.  Rounds: 4000 pairs at seed 7.  Games: 3000 pairs
(`mc:128`: 2000) at each of seeds 7 and 8, pooled to 12 000 (`mc:128`:
8000) games.  Parenthesized ranges are 95% intervals, in percent.
`scripts/bench-panel.sh` runs exactly that panel and prints the table
below, so every number here is reproducible from the commit it was taken
at — the arena is deterministic in its seed.

| Bot vs baseline | Rounds won        | Points/round | Games won         |
|-----------------|-------------------|--------------|-------------------|
| `greedy`        | 39.4% (38.4–40.5) | 8.92 vs 8.29 | 59.7% (58.9–60.6) |
| `mc:64`         | 51.5% (50.4–52.6) | 9.22 vs 8.41 | 54.8% (53.9–55.7) |
| `mc:128`        | 52.7% (51.6–53.8) | 9.90 vs 8.38 | 59.6% (58.5–60.7) |

The default heuristic concedes rounds by design — it hunts gin while the
baseline knocks at the first opportunity — yet wins the matches on the
gin and undercut bonuses; the EAAI-21 literature identifies exactly that
patient, undercutting style as the strongest exploit of this baseline's
knock-ASAP habit.  The Monte Carlo bots win rounds instead, and the
comparison is not transitive: `mc:64` beats `greedy` head-to-head over
whole games (53.0% of 12 000 paired games, p < 0.001) yet exploits the
baseline less than `greedy` does, while doubling the sample count closes
that gap — `mc:128` matches `greedy` against the baseline.  For
calibration, EAAI-21 entries reported roughly 55–68% against this same
baseline under the same dealer protocol (metrics vary by paper).
Throughput in those runs, trials fanned across 16 cores: ~19 500 games/s
for `greedy` vs the baseline, 7–8 games/s at `mc:64`, ~4.9 at `mc:128`.

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

## Alternatives

No other open-source project ships gin rummy bots as a reusable library.
[OpenSpiel] and [RLCard] embed gin rummy environments for generic search
and reinforcement-learning algorithms (bring your own agent), and
[gin-rummy-eaai] is the EAAI-2021 challenge framework whose reference
baseline this crate ports as [`EaaiSimpleBot`] — precisely so the numbers
above stay comparable with the challenge literature.

[gin rummy]: https://www.pagat.com/rummy/ginrummy.html
[OpenSpiel]: https://github.com/google-deepmind/open_spiel
[RLCard]: https://github.com/datamllab/rlcard
[gin-rummy-eaai]: https://github.com/tneller/gin-rummy-eaai
[`EaaiSimpleBot`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.EaaiSimpleBot.html
[gin-rummy]: https://crates.io/crates/gin-rummy
[`Strategy`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/trait.Strategy.html
[`View`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.View.html
[`Table`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.Table.html
[`HeuristicBot`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.HeuristicBot.html
[`MonteCarloBot`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.MonteCarloBot.html
[`McConfig`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/struct.McConfig.html
[`Round`]: https://docs.rs/gin-rummy/latest/gin_rummy/round/struct.Round.html
