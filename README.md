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
the `gin-rummy-arena/v1` schema and reproducibility metadata. Exact-sign
p-values also have `*_decimal` scientific-string fields; their numeric fields
are `null`, never a false zero, when the positive value is below `f64` range.

The fixed EAAI-baseline panel uses the corrected protocol, pair-cluster
intervals, and raw target-reaching scores.  Round diagnostics use seed 7;
each game row pools seeds 7 and 8.  It used 4000 mirrored round pairs,
3000 game pairs per seed for `greedy` and `mc:64`, and 2000 game pairs per
seed for `mc:128`.  Exact pair-sweep sign-test p-values are below .001.

| Bot vs baseline | Decisive rounds won | Points/round | Games won | Raw score/game |
|-----------------|---------------------:|-------------:|----------:|---------------:|
| `greedy`        | 39.4% (38.5–40.4%) | 8.92 vs 8.29 | 59.8% (59.0–60.6%) | 90.28 vs 78.76 |
| `mc:64`         | 46.1% (45.2–47.1%) | 11.48 vs 8.51 | 65.4% (64.6–66.2%) | 92.52 vs 70.67 |
| `mc:128`        | 47.1% (46.1–48.1%) | 12.29 vs 8.36 | 69.2% (68.3–70.2%) | 94.93 vs 66.87 |

The heuristic still concedes decisive rounds by hunting gin while the
baseline knocks at the first opportunity, yet wins whole games on raw score.
`mc:64` also beats `greedy` head-to-head in 63.6% (62.8–64.4%) of 12,000
games, 92.94–71.62 raw score/game, with exact pair-sweep p < .001.  EAAI-21
entries reported roughly 55–68% against the baseline, but metrics and host
semantics vary, so comparisons require the protocol qualifications above.

### Strong opponents

The fixed strong-opponent panel used 6,000 mirrored game pairs per matchup
(12,000 games) under the corrected EAAI protocol.  Every seed agreed in
direction and all pooled Holm-adjusted exact p-values are below .001.

| Candidate | Opponent | Candidate games won (pair-cluster 95% CI) | Finding |
|-----------|----------|-------------------------------------------:|---------|
| `greedy` | `gold-paper` | 62.2% (61.4–62.9%) | candidate edge |
| `mc:64` | `gold-paper` | 69.5% (68.8–70.3%) | candidate edge |
| `mc:128` | `gold-paper` | 74.5% (73.8–75.2%) | candidate edge |
| `greedy` | `marjj-v5-surrogate` | 29.2% (28.4–30.0%) | opponent edge |
| `mc:64` | `marjj-v5-surrogate` | 42.4% (41.6–43.2%) | opponent edge |
| `mc:128` | `marjj-v5-surrogate` | 46.7% (45.9–47.5%) | opponent edge |

These are controlled host-engine comparisons, not executions of the original
agents or reproductions of their tournaments.  Gold's published 70–99% came
from a different single-hand environment, and its exactness covers meld
decomposition rather than game-theoretically optimal full-game play.  The
later public MARJJ v5 file is only a surrogate; it is not established as the
2021 champion binary.  Settlement and layoffs use host adaptations, and no
EAAI 30-second player timer is enforced.  See the
[strong-opponent report](docs/strong-opponents.md) and
[raw JSON](docs/strong-opponents.json) for per-seed results, round diagnostics,
provenance, conformance, and all adaptation details.

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
