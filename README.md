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

Game-level protocol is explicit too: [`eaai_rules`] returns the challenge's
exact scoring preset (no Big Gin, box, game, or shutout bonus), while
[`DealerRotation`] tells `Table` and score-aware strategies how the next
dealer is chosen.  The EAAI variant flips the dealer after a scored hand and
retains the same dealer after a dead hand.

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
- [`EaaiSimpleBot`] (feature `rand`): the published policy of
  `SimpleGinRummyPlayer`, the baseline every entry of the EAAI-2021 Gin Rummy
  AI challenge was measured against.  Deliberately weak and knob-free — it
  exists as a cross-engine yardstick when the host's policy adaptations,
  scoring preset, and dealer protocol are reported explicitly.

## Benchmarks

Correct EAAI-compatible measurements use `--rules eaai` and
`--alternate-dealer`.  The latter means *alternate after a scored hand*;
a dead hand is redealt by the same dealer.  Arena trials are mirrored
pairs, with the bots swapping seats under common-random-number deal streams.
For a single round the deal is identical.  During a whole game, one
orientation can score where the other goes dead, so their later dealer
sequences can diverge even though they continue from identically seeded
shuffle streams.

Headline uncertainty for mirrored runs is a 95% pair-cluster confidence
interval.  The primary comparison is the exact two-sided sign test over
pairs swept by each bot; the normal paired-z value is diagnostic only.
Use `--seeds 7,8 --format json` to obtain per-seed and pooled results with
the `gin-rummy-arena/v1` schema and reproducibility metadata.

The panel below is retained as **historical data only**.  It predates the
correction that retains the dealer after dead EAAI hands and the exact EAAI
scoring preset, and its intervals use the former analysis.  It therefore
needs a corrected-protocol regeneration before any rate, interval,
p-value, or throughput figure is quoted as current.  A fresh
`scripts/bench-panel.sh` run will replace it; the cells are left unchanged
until that run completes.

| Bot vs baseline | Rounds won        | Points/round | Games won         |
|-----------------|-------------------|--------------|-------------------|
| `greedy`        | 39.4% (38.4–40.5) | 8.92 vs 8.29 | 59.7% (58.9–60.6) |
| `mc:64`         | 51.5% (50.4–52.6) | 9.22 vs 8.41 | 54.8% (53.9–55.7) |
| `mc:128`        | 52.7% (51.6–53.8) | 9.90 vs 8.38 | 59.6% (58.5–60.7) |

Historical interpretation, not a current strength claim: the old panel
suggested that the default heuristic conceded rounds by hunting gin while
the baseline knocked at the first opportunity, yet won matches on gin and
undercut bonuses.  It also reported `mc:64` beating `greedy` head-to-head
over whole games (53.0% of 12 000 paired games, p < 0.001), despite
exploiting the baseline less, and `mc:128` matching `greedy` against the
baseline.  EAAI-21 entries reported roughly 55–68% against this baseline
(metrics vary by paper), but the old panel's protocol mismatch prevents a
direct comparison.
Throughput in those runs, trials fanned across 16 cores: ~19 500 games/s
for `greedy` vs the baseline, 7–8 games/s at `mc:64`, ~4.9 at `mc:128`.

### Strong opponents

The strong-opponent harness has adapters for two external reference agents.
No result is claimed until the corrected-protocol panel has completed; see
the [strong-opponents report](docs/strong-opponents.md) for methodology,
provenance, conformance checks, and eventual measurements.

| Opponent | Reference | Corrected-protocol result |
|----------|-----------|---------------------------|
| GoldStandardAgent host adaptation (`gold-paper`) | 2026 Adversarial Co-Evolution reference; exact meld decomposition, not a game-theoretically optimal full-game player | Pending |
| MARJJ v5 host surrogate (`marjj-v5-surrogate`) | Public repository associated with the 2021 challenge winner; the v5 file is not established as the submitted championship build | Pending |

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
- `arena`: bot-vs-bot tournaments with win-rate statistics.  For example:

  ```console
  cargo run --release --example arena -- --games 3000 \
    --p1 mc --p2 gold-paper --rules eaai --alternate-dealer \
    --seeds 7,8 --format json
  ```

  Bare `mc` and `mca` use 128 samples; use `mc:N` or `mca:N` to set an
  explicit budget.

## Alternatives

No other open-source project ships gin rummy bots as a reusable library.
[OpenSpiel] and [RLCard] embed gin rummy environments for generic search
and reinforcement-learning algorithms (bring your own agent), and
[gin-rummy-eaai] is the EAAI-2021 challenge framework whose reference
baseline this crate ports as [`EaaiSimpleBot`] — so corrected-protocol
measurements can be calibrated against the challenge literature.

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
[`DealerRotation`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/enum.DealerRotation.html
[`eaai_rules`]: https://docs.rs/gin-rummy-engine/latest/gin_rummy_engine/fn.eaai_rules.html
[`Round`]: https://docs.rs/gin-rummy/latest/gin_rummy/round/struct.Round.html
