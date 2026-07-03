---
name: sync-sim
description: Mirror a change in the gin-rummy mechanics crate (Rules, Round state machine, scoring, phases) into this crate's forward model, knowledge tracking, and driver. Use whenever ../gin-rummy changed or its dependency version bumps, before touching anything else in this crate.
---

# Sync the forward model with gin-rummy

`MonteCarloBot` rolls determinized worlds out on `Sim` (`src/sim.rs`), a
crate-private replica of `gin_rummy::Round`, because a `Round` cannot be
constructed mid-game.  The replica duplicates the round rules on purpose;
this procedure keeps the duplicate honest.

## 1. Read the upstream change

```console
git -C ../gin-rummy log --oneline -10
git -C ../gin-rummy diff <old>..<new> -- src/round.rs src/rules.rs src/game.rs
```

Also read the gin-rummy CHANGELOG.md entry if one exists.

## 2. Mirror by decision table

| Upstream change | Local edits |
| --- | --- |
| Draw/shed cycle, phase flow | `Sim::rollout` and `SimPhase` (`src/sim.rs`), `Table::step` (`src/driver.rs`), `View::can_take_discard` (`src/view.rs`) |
| Upcard offer / pass rules | `Sim::pass`, the `passes` initialization in `MonteCarloBot::sim` (`src/mc.rs`), the driver's `forced_stock` handling |
| Dead-hand rule (currently: the round is dead when a discard leaves 2 stock cards) | `Sim::discard` |
| Knock, gin, undercut settlement, layoffs | `Sim::knock`, `Sim::big_gin` |
| New or changed `Rules` fields | `Sim` gating (see how `big_gin_bonus` gates `Sim::rollout`), possibly `HeuristicBot` and `MonteCarloBot` decisions |
| Scoring or `RoundResult` variants | `score` in `src/mc.rs`; the tallies in `examples/arena.rs` and narration in `examples/play.rs` |
| Newly visible information | `Knowledge` and `View` accessors (`src/view.rs`), the bookkeeping ledger in `Table::step` (see the invariants in CLAUDE.md), `tests/view.rs` |
| A new action or decision point | `src/action.rs`, the `Strategy` trait, `Table::step`, both bots, `Sim` |

## 3. Verify

The equivalence proptest `sim_matches_round_on_greedy_selfplay` in
`src/sim.rs` replays whole greedy self-play rounds through both models:
same deal in, same result out.  Run it hard, then everything:

```console
cargo test --all-features sim_
PROPTEST_CASES=4096 cargo test --release --all-features sim_matches_round
cargo test --all-features
```

Rules of engagement:

- Never weaken the proptest or special-case a mismatch.  A disagreement
  means `Sim` is wrong or you misread the upstream change — there is no
  third possibility worth coding around.
- Commit any `*.proptest-regressions` file a failure produces.
- If the change alters strategy-relevant values (scores, knock limits,
  bonuses), finish with the `measure-strength` skill: a rollout change
  shifts every Monte Carlo evaluation.
