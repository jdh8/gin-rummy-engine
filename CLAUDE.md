# gin-rummy-engine

This crate implements gameplay strategy for gin rummy on top of the
[gin-rummy](../gin-rummy) mechanics crate: a `Strategy` trait, an
information-hygienic `View`, a `Table` driver, a deterministic
`HeuristicBot`, and a determinized `MonteCarloBot` (feature `rand`).
Game mechanics — card types, the deadwood solver, the `Round` state machine,
scoring — live in gin-rummy and are out of scope here; only decision-making
belongs in this crate.

## Map of the crate

| Path | Contents |
| ---- | -------- |
| `src/strategy.rs` | The `Strategy` trait: four decision methods against a `View`, object-safe. |
| `src/action.rs` | Per-phase action types, so a structurally illegal action cannot be expressed. |
| `src/view.rs` | `View` (public, includes `game_scores` and `dealer_rotation`) plus the crate-private `Knowledge` the driver accumulates. |
| `src/driver.rs` | `Table`, `play_round`, `play_game`, `EngineError`: validates and applies decisions, keeps both seats' `Knowledge` current, and supplies the public dealer protocol to strategies. |
| `src/protocol.rs` | Public `DealerRotation` and the exact EAAI challenge scoring preset, `eaai_rules()`. |
| `src/heuristic.rs` | `HeuristicBot`, `HeuristicConfig`, and the shared greedy primitives `best_shed`, `improves`, `greedy_layoff`. |
| `src/mc.rs` | `MonteCarloBot` (feature `rand`): plausibility-biased world sampling, common random numbers, a game-winning equity objective, significance-gated deviation from the greedy baseline, batched rollouts that eliminate statistically hopeless challengers early. |
| `src/sim.rs` | Crate-private forward model for rollouts (feature `rand`); must mirror `gin_rummy::round` exactly. |
| `src/value.rs` | Crate-private game-win value function (feature `rand`): checked-in greedy-self-play outcome models, solved by DP for each `(Rules, DealerRotation)`.  Backs the default `GameValue::Table` equity; unbaked rulesets fall back to affine. |
| `tests/view.rs` | Information-hygiene assertions on driven rounds. |
| `tests/driver.rs` | End-to-end rounds and games, illegal-action reporting and retry. |
| `tests/proptest.rs` | Termination, deck partition, and the `unseen` identity under every ruleset. |
| `tests/strength.rs` | Statistical strength tripwire, `#[ignore]`d; release mode only. |
| `benches/decision.rs` | Criterion benches for per-decision latency. |
| `examples/play.rs` | Human vs bot in the terminal. |
| `examples/arena.rs` | Bot-vs-bot tournaments with mirrored pairs, pair-cluster intervals, exact pair-sweep sign tests, multi-seed pooling, and versioned JSON output. |
| `examples/support/strong/` | Benchmark-only Gold paper and MARJJ v5 surrogate adaptations shared by the arena, the tuning sweep, and integration tests; never expose them through the public API or interactive bot list. |
| `examples/baseline_report.rs` | Validates the fixed EAAI-baseline JSON legs and emits README-ready pair-aware results. |
| `examples/strong_report.rs` | Validates the fixed strong-opponent evidence bundle and generates `docs/strong-opponents.{md,json}`; publication requires pinned passing conformance. |
| `examples/marjj_diagnose.rs` | Replays the default and calibrated-hand `mc:128` arms against MARJJ on common random numbers and decomposes their score margin by finish channel without changing the arena schema. |
| `examples/tune.rs` | Whole-game A/B self-play sweep for tuning the heuristic's and Monte Carlo's knobs against a fixed opponent — including the benchmark-only strong adaptations, under either dealer protocol (`--alternate-dealer`). |
| `scripts/bench-panel.sh` | Regenerates README's corrected-EAAI baseline panel with pair-cluster intervals, exact sweep tests, and raw scores. |
| `scripts/bench-strong.sh` | Runs the predeclared strong-opponent smoke and fixed panels without nested Monte Carlo parallelism. |
| `scripts/check-strong-conformance.sh` | Compares the native adaptations with user-supplied pinned upstream source trees; it never downloads or vendors them. |

## Measured reference results

The fixed corrected-EAAI baseline panel reports game win shares of 59.8%
(59.0–60.6%) for `greedy`, 65.4% (64.6–66.2%) for `mc:64`, and 69.2%
(68.3–70.2%) for `mc:128` against `EaaiSimpleBot`.  `mc:64` beats `greedy`
head-to-head at 63.6% (62.8–64.4%).  Exact mirrored-pair sweep p-values are
below .001 for all four comparisons.

The fixed strong panel reports candidate game win shares against `gold-paper`
of 62.2% (`greedy`), 69.5% (`mc:64`), and 74.5% (`mc:128`); against
`marjj-v5-surrogate` they are 29.2%, 42.4%, and 46.7%.  Every seed agrees in
direction and every pooled Holm-adjusted exact p-value is below .001.  These
are host-engine adaptation results, not original-agent tournament
reproductions.  Keep the Gold and MARJJ qualifications in
[`docs/strong-opponents.md`](docs/strong-opponents.md) attached to every use
of these figures; raw evidence is in
[`docs/strong-opponents.json`](docs/strong-opponents.json).

## Invariants

Check these before merging any change; each names its guarding test.

1. **Information hygiene.**  `Round` exposes both hands and the stock
   order, but strategies only ever receive a `View`, whose accessors are
   the whitelist of legally visible information.  Never add a `View`
   accessor that leaks the opponent's hand, the stock order, or the
   wrapped `Round`; never hand a `Round` or `Table` to a `Strategy`.
   Guarded by `tests/view.rs`.
2. **The unseen identity.**  Until a knock reveals the spread,
   `unseen.len() == stock_len + opponent_hand_len − opponent_known.len()`
   — exactly the cards a determinizing bot distributes between the stock
   and the hidden part of the opponent's hand.  After a knock the spread
   also counts as seen and the identity intentionally breaks.  Guarded by
   `tests/proptest.rs` and `tests/view.rs`.
3. **Driver bookkeeping.**  Every action applied in `Table::step` updates
   `Knowledge` for both seats.  The current ledger: a take sets the
   actor's `taken_discard` and inserts into the observer's
   `opponent_known`; a pass inserts into the observer's `opponent_passed`,
   and the second pass sets the non-dealer's `forced_stock`; a stock draw
   clears the actor's `forced_stock`; a shed clears the actor's
   `taken_discard` and moves the card from the observer's `opponent_known`
   to `opponent_shed`; a layoff removes the card from the observer's
   `opponent_known` (it is public on the spread).  Extending an action
   means extending this ledger and `tests/view.rs` together.
4. **`Sim` mirrors `Round`.**  A `Round` cannot be constructed mid-game,
   so Monte Carlo rollouts run on the crate-private `Sim` replica in
   `src/sim.rs`.  Any mechanics change upstream must be mirrored there;
   the equivalence proptest `sim_matches_round_on_greedy_selfplay` (in
   `src/sim.rs`) replays whole greedy self-play rounds through both models
   and must keep passing *unweakened*.  Follow the `sync-sim` skill.
   `src/value.rs`'s checked-in outcome models are *measured through*
   `Sim::rollout`, so a mechanics change silently invalidates them too:
   `baked_matches_fresh_sampling` is the guard, and `regenerate_baked`
   (`#[ignore]`d) reprints all four constants when the change is
   deliberate.  Regenerating them changes every Monte Carlo evaluation,
   so re-measure afterwards.
5. **The greedy core doubles as the rollout policy.**  `best_shed`,
   `improves`, `joins_a_meld`, and `greedy_layoff` in `src/heuristic.rs`
   are shared with `Sim::rollout` (and `joins_a_meld` with
   `EaaiSimpleBot`, so the modeled opponent and the real baseline cannot
   drift).  Under `SeatPolicy::default()` the rollout plays exactly
   `HeuristicBot` with `knock_threshold: u8::MAX, safety_weight: 0` on
   both seats, and the equivalence proptest pins that policy.  The
   `McConfig` rollout knobs (`rollout_knock_self`,
   `rollout_knock_opponent`, `opponent_model`) bend it per seat, and
   `McConfig::default()` does bend it: the bot's own continuations knock
   only on gin (`rollout_knock_self: 0`) while the modeled opponent still
   knocks at the first legal chance.  Changing the shared
   functions changes both bots and shifts every Monte Carlo evaluation,
   and changing any `McConfig` default is a strength change — either way,
   re-measure afterwards (follow the `measure-strength` skill).
6. **Dealer protocol is part of the position.**  `Table` defaults to
   `DealerRotation::WinnerDeals`.  EAAI-compatible games use
   `eaai_rules()` with `DealerRotation::AlternateAfterScoredRound`: a
   scored hand flips the dealer and a dead hand retains it.  `View`
   exposes this public state, and the Monte Carlo game-value cache and DP
   are keyed by `(Rules, DealerRotation)`.  Never describe EAAI as
   alternating after a dead hand, and test both rotations when changing
   the driver, view, or score-aware evaluation.
7. **Determinism.**  `HeuristicBot` is a pure function of the view;
   `MonteCarloBot` owns its RNG, so a seeded generator replays
   identically.  Tests rely on both.  Never call a global RNG inside a
   strategy — take the generator as a constructor argument.  Arena
   mirroring is a common-random-number seat swap: rounds clone the exact
   deal, while games reuse identically seeded shuffle streams.  Do not
   claim that every later dealer/deal pairing stays identical in a game;
   if one orientation has a dead hand where the other scores, their dealer
   sequences can diverge.

## The sibling crate

- gin-rummy is a normal crates.io dependency by default.  Switch it to a
  `path = "../gin-rummy"` dependency only while developing a coordinated
  change that needs unreleased gin-rummy commits, and switch it back to a
  version requirement before merging.
- Types to know: `Card`, `Hand` (a 52-card bitset with `|`, `&`, `-`),
  `Meld`, `Melds`, `Round`, `Rules` (presets `new`/`classic`/`palace`),
  `Game`, `RoundResult`, and the solver functions `deadwood` and
  `best_melds`.  Ranks are ace-LOW (A = 1, K = 13) because gin runs are
  A-2-3 and never Q-K-A.
- `Hand` parses from dotted suit groups ordered clubs.diamonds.hearts.spades:
  `"A23.456.789.5K"` is ♣A♣2♣3 ♦4♦5♦6 ♥7♥8♥9 ♠5♠K.  Cards parse leniently:
  `S10`, `♠10`, `st`, and `♠T` all name the ten of spades.
- For rules questions, [Pagat](https://www.pagat.com/rummy/ginrummy.html)
  is the most reliable source; scoring bonuses vary by rule school and are
  all knobs on `Rules`.
- Do not construct an EAAI approximation from `Rules::new()`.  Use the
  engine's public `eaai_rules()` preset: no Big Gin, box, game, or shutout
  bonus, alongside the scored-hand-only dealer rotation above.

## Verification

Run the same gauntlet CI runs (`.github/workflows/rust.yml`):

```console
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo test --all-features
cargo test
cargo check --no-default-features
```

The plain `cargo test` leg matters: `--all-features` turns on `parallel`,
so only the default-features run exercises the serial Monte Carlo scoring
path (CI runs both).

- MSRV is 1.93 (edition 2024) and CI tests it explicitly; avoid newer
  language or standard-library features.
- CI pins direct dependencies to their declared minimum versions (nightly
  `-Z direct-minimal-versions`); when using a new dependency API, make
  sure the floor declared in `Cargo.toml` really provides it.
- The no-default-features build must stay dependency-light: the trait,
  view, driver, and heuristic bot cannot touch `rand`.
- After changing any bot's decision logic, run the strength tripwire —
  release mode, never debug:
  `cargo test --release --test strength -- --ignored` (minutes long).
  For real measurement, follow the `measure-strength` skill.
- For a publishable arena panel, pass explicit `--seeds`, request
  `--format json`, and retain the `gin-rummy-arena/v1` output.  Bare `mc`
  and `mca` mean 128 samples, although publication commands should spell
  out `mc:128` or `mca:128`.  Use pair-cluster confidence intervals and
  the exact pair-sweep sign test as headline inference; paired-normal
  z-values are diagnostics only.
- For performance-sensitive changes, `cargo bench` (needs the default
  `rand` feature).

## Conventions

- Every public item carries a doc comment (`#![warn(missing_docs)]` plus
  clippy `-D warnings` in CI make a missing one a build failure).  Docs
  are prose: complete sentences that explain *why*, not just *what*; two
  spaces after sentence-ending periods; hand-wrapped near 76 columns.
  Fallible public functions get an `# Errors` section.
- Comments state constraints the code cannot: which rule a branch
  implements, why a bound holds.  No narration of the obvious.
- API habits: `#[must_use]` on pure constructors and accessors, `const fn`
  where possible, `#[non_exhaustive]` on types that will grow
  (`HeuristicConfig`, `EngineError`), builder-style consuming setters
  (`MonteCarloBot::samples`).
- Tests: deterministic fixtures (`fixed_deal`, the sorted deck dealt
  round-robin), `expect` messages that read as assertions ("a partitioned
  deck"), proptest for whole-round properties.  Commit any
  `*.proptest-regressions` file that a failure produces.
- CHANGELOG.md follows Keep a Changelog: entries describe the impact on
  users of the crate, not implementation internals.
- Commit messages: imperative subject, then a body in full prose
  summarizing design and measured impact (see `git log` for the house
  style).

## Recipes

Step-by-step procedures live as project skills in `.claude/skills/`;
follow them instead of improvising, and update them in the same commit as
any change that invalidates them:

- **sync-sim** — mirror a gin-rummy mechanics change into `Sim`,
  `Knowledge`, and the driver.
- **measure-strength** — evaluate bot changes statistically without
  fooling yourself.
- **add-strategy** — everything a new `Strategy` implementation must
  touch.
- **release** — version, changelog, tag, and the publish-order constraint
  with gin-rummy.

## After updating the codebase

- Format the code with `cargo fmt`.
- Run the whole verification gauntlet above and fix everything it flags.
- Update [CHANGELOG.md](CHANGELOG.md) with a summary of the changes and
  their impact on users.
- Propose a clear and descriptive commit message.
