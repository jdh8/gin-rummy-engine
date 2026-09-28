# Performance backlog

Ideas for making `MonteCarloBot` decide faster, ranked by expected payoff
per line of diff.  A Monte Carlo turn is pure rollout cost: the saved
Criterion runs put a greedy self-play round at about 0.06 ms and an
`mc:512` discard decision at about 56 ms before these optimizations.
Rollout turns originally repeated solves between the draw, shed, and
knock decisions.  Everything below shrinks either the number of solves
or the cost of one.

Items marked **re-measure** change decisions or the sampled distribution
and must go through the `measure-strength` skill before being claimed as
an improvement.  Items marked **pure reuse** must leave the
`sim_matches_round_on_greedy_selfplay` proptest passing unweakened.

## 1. Build flags (measured, zero code)

The deadwood solver lives on `count_ones` and `trailing_zeros`, and the
default x86-64 target emits neither `popcnt` nor `tzcnt`.  Measured on the
`monte carlo turn, 128 samples` bench, 2026-09-28, load average 1.6 on 16
cores:

| Build | Mean |
| ----- | ---: |
| previous default release profile | 8.05 ms |
| `-C target-cpu=native`, `lto = "fat"`, `codegen-units = 1` | 6.85 ms |

Applied: `[profile.release]` now sets `lto = "fat"` and
`codegen-units = 1`.  Keep `target-cpu=native` in an untracked local
Cargo config (such as `~/.cargo/config.toml`) only, so shared builds
remain portable.  Downstream applications choose their own release
profile; this crate's profile applies when building this repository.

## 2. Delete redundant solves in the rollout (implemented, pure reuse)

Applied in `src/sim.rs`, `Sim::rollout_observed`, with the shared
`improving_melds` helper in `src/heuristic.rs`:

- Carry the successful draw check's eleven-card arrangement into the
  shed instead of solving the same hand again.
- Cache each seat's residual deadwood from its previous shed for its
  next draw check.  Both caches live inside the rollout, so resuming
  from an arbitrary phase cannot inherit stale values.
- Reuse the knock spread when shedding an unmelded card, subtracting
  that card from the settlement deadwood.  A melded discard still
  requires a fresh arrangement.

The original `Sim`/`Round` equivalence test remains unchanged.  Additional
properties compare exact knock spreads and uncached rollout traces under
both draw policies, varying knock thresholds, and mid-round resumes.

Measured on 2026-09-28 against `95e3c7b`, with the same release profile,
local `target-cpu=native` flag, and both executables pinned to CPU 2:

| Decision | Before | After | Time reduction |
| -------- | -----: | ----: | -------------: |
| `mc:128` | 6.643 ms | 6.155 ms | 7.3% |
| `mc:512` | 27.773 ms | 25.722 ms | 7.4% |

Criterion used 100 measurements per case; both improvements had
`p < 0.05`.  The draw helper must be inlined: returning its arrangement
out of line erased the savings and regressed this benchmark.
Reproduce by saving the baseline before the change, then comparing:

```console
taskset -c 2 cargo bench --bench decision -- 'monte carlo turn, (128|512) samples' --save-baseline rollout-before
taskset -c 2 cargo bench --bench decision -- 'monte carlo turn, (128|512) samples' --baseline rollout-before
```

## 3. Sampling overhead (measured; deadwood-only swaps rejected)

`MonteCarloBot::develop` only runs with `hand_calibration: true`, which
is **off by default**.  Default sampling selects the lowest-deadwood of
`max(1, opponent_strength(pile_len) * 200 / 100)` uniform hands per world;
there is no 64-attempt development loop in a default decision.  At pile
lengths 1, 12, and 24, this costs 2, 12, and 24 solves per world.

Measured on 2026-09-28 against `72f3779`, using the release profile and
local `target-cpu=native`, on a Ryzen 7 8700F pinned to CPU 2.  The ignored
`sampling_overhead_measurement` test isolates `sample_worlds` from
rollouts and times 100 batches of 512 worlds with `StdRng` seed 3.
Fixtures are the first live ten-card-opponent positions at each pile
length found by seeded, gin-only greedy self-play; they have 0, 0, and 4
known opponent cards.  These are three diagnostic positions, not an
average over games.  Diagnostic deadwood solves are outside the timer.

The prototype cached `best_melds(known | hidden)`, picked outgoing cards
from `arrangement.deadwood_cards() & hidden` (falling back to all hidden
cards when empty), and retained the new arrangement only on an accepted
swap.  The 64-attempt cap and target-distance acceptance stayed unchanged.
Five runs per binary alternated baseline/candidate order; the table gives
median milliseconds per 512 worlds:

| Pile length | Default sampler | Calibrated sampler | Calibrated prototype |
| ----------: | --------------: | -----------------: | -------------------: |
| 1 | 0.330 | 2.090 | 2.250 |
| 12 | 1.427 | 5.006 | 4.576 |
| 24 | 2.457 | 4.787 | 4.979 |

The prototype was about 9% faster mid-round, but slower in the opening
and late fixtures.  Its unchanged default path measured 0.340, 1.485,
and 2.589 ms, so small timing differences also include build/runtime
variation.  More importantly, target fit was not uniformly better:

| Pile length | Target | Mean absolute target error, before → prototype | Exact target share, before → prototype |
| ----------: | -----: | --------------------------------------------: | -------------------------------------: |
| 1 | 50 | 0.106 → 0.695 | 89.80% → 87.44% |
| 12 | 13 | 1.182 → 0.461 | 50.86% → 76.99% |
| 24 | 4 | 0.643 → 0.734 | 56.18% → 43.03% |

Preserving melds restricts how an already-too-strong hand can move back
up toward its target.  The existing mid-round calibration test passes,
but alone would miss these opening and late-round changes.  Reject the
prototype: it does not speed up the default bot and is not a consistent
calibrated-sampling improvement.  Production sampling and defaults stay
unchanged.  No strength panel was rerun and no strength claim follows
from this experiment; a future sampler candidate still owes calibration
checks and the `measure-strength` procedure before adoption.

Reproduce the retained baseline diagnostic:

```console
taskset -c 2 cargo test --release --lib sampling_overhead_measurement -- --ignored --nocapture
```

## 4. Flatten the parallel loop (implemented, pure reuse)

`score_worlds` (`src/mc.rs`) now schedules one indexed parallel iterator
over the active candidate × world product per growing batch, instead of
one fork-join per candidate.  Results remain contiguous per candidate
and ordered by world; the sequential reduction and elimination
checkpoints are unchanged.  The serial build still evaluates lazily
without allocating a batch-result buffer.

The scoring regression compares every candidate's equities and summed
round points against its serial prefix, including eliminated candidates,
with 1, 32, 33, 97, and 256 worlds.  The seeded-pick test and the original
`sim_matches_round_on_greedy_selfplay` property remain unchanged.

Measured on 2026-09-28 against `4ed4721`, with the same release profile
and local `target-cpu=native` flag, on a Ryzen 7 8700F.  Both runs used
eight Rayon threads pinned to CPUs 0–7 (eight physical cores).  Criterion
used 100 measurements per case; these are mean decision times:

| Decision | Before | After | Change |
| -------- | -----: | ----: | -----: |
| `mc:128` | 1.430 ms | 1.230 ms | 14.0% faster (`p < 0.05`) |
| `mc:512` | 5.592 ms | 5.634 ms | No significant change (`p = 0.71`) |

This measures one discard fixture, not average whole-game throughput.
No decision logic or sampling changed, so no strength panel was rerun.
Reproduce by saving the baseline before the change, then comparing:

```console
RAYON_NUM_THREADS=8 taskset -c 0-7 cargo bench --features parallel --bench decision -- 'monte carlo turn, (128|512) samples' --save-baseline parallel-before
RAYON_NUM_THREADS=8 taskset -c 0-7 cargo bench --features parallel --bench decision -- 'monte carlo turn, (128|512) samples' --baseline parallel-before
```

## 5. Early acceptance (re-measure)

`score_worlds` eliminates a challenger once the incumbent beats it at the
`gate_z` margin, but a challenger that clearly beats the incumbent still
rolls the full sample count.  Accepting at the same gate symmetrically
would end clear decisions early.  This is a decision change.

## 6. Allocations (last)

Every rollout clones the stock and the pile into two `Vec`s.  Replacing
them with `[Card; 52]` plus a length saves at most a few percent.  Do this
only after a profile shows the allocator in the top frames.

## Skipped

A per-decision solver memo keyed by the 64-bit hand bits.  Hit rates are
unknown; add a debug counter first and build the memo only if hits are
common.
