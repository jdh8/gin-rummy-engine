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

## 3. Sampling overhead (re-measure)

`MonteCarloBot::develop` (`src/mc.rs`) spends up to 64 solves per world,
and each strength draw costs another, so at 512 samples tens of thousands
of solves run before any rollout starts.  Picking the outgoing card from
`deadwood_cards()` instead of the whole hand makes swaps land more often
and reach the target in fewer attempts.  This changes the sampled hidden
hands, so the calibration tests and the strength panels must be rerun.

## 4. Flatten the parallel loop

`score_worlds` (`src/mc.rs`) launches one `par_iter` per candidate per
32-world batch, which is too little work per fork-join.  One `par_iter`
over the candidate × world product per batch keeps the sequential
world-order reduction, so serial and parallel builds still decide
identically, and gives rayon a real chunk of work.

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
