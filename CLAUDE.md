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
| `src/view.rs` | `View` (public) plus the crate-private `Knowledge` the driver accumulates. |
| `src/driver.rs` | `Table`, `play_round`, `play_game`, `EngineError`: validates and applies decisions, keeps both seats' `Knowledge` current. |
| `src/heuristic.rs` | `HeuristicBot`, `HeuristicConfig`, and the shared greedy primitives `best_shed`, `improves`, `greedy_layoff`. |
| `src/mc.rs` | `MonteCarloBot` (feature `rand`): plausibility-biased world sampling, common random numbers, significance-gated deviation from the greedy baseline. |
| `src/sim.rs` | Crate-private forward model for rollouts (feature `rand`); must mirror `gin_rummy::round` exactly. |
| `tests/view.rs` | Information-hygiene assertions on driven rounds. |
| `tests/driver.rs` | End-to-end rounds and games, illegal-action reporting and retry. |
| `tests/proptest.rs` | Termination, deck partition, and the `unseen` identity under every ruleset. |
| `tests/strength.rs` | Statistical strength tripwire, `#[ignore]`d; release mode only. |
| `benches/decision.rs` | Criterion benches for per-decision latency. |
| `examples/play.rs` | Human vs bot in the terminal. |
| `examples/arena.rs` | Bot-vs-bot tournaments with Wilson score intervals. |

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
5. **The greedy core doubles as the rollout policy.**  `best_shed`,
   `improves`, and `greedy_layoff` in `src/heuristic.rs` are shared with
   `Sim::rollout`; the rollout plays exactly `HeuristicBot` with
   `knock_threshold: u8::MAX, safety_weight: 0`.  Changing these functions
   changes both bots and shifts every Monte Carlo evaluation — re-measure
   afterwards (follow the `measure-strength` skill).
6. **Determinism.**  `HeuristicBot` is a pure function of the view;
   `MonteCarloBot` owns its RNG, so a seeded generator replays
   identically.  Tests rely on both.  Never call a global RNG inside a
   strategy — take the generator as a constructor argument.

## The sibling crate

- gin-rummy is a **path dependency**: `../gin-rummy` must be checked out
  next to this repository (CI clones `jdh8/gin-rummy` there).  Coordinated
  changes need commits in both repositories.
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

## Verification

Run the same gauntlet CI runs (`.github/workflows/rust.yml`):

```console
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo test --all-features
cargo check --no-default-features
```

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
