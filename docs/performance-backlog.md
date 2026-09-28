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

## 5. Early acceptance (measured; first-crossing acceptance rejected)

`score_worlds` eliminates a challenger once the incumbent beats it at the
`gate_z` margin.  The prototype also stopped at the first batch boundary
where any surviving challenger beat the incumbent, then used the existing
`recommended` ranking.  Immediately after `alive.retain`, it replaced the
stop condition with:

```rust
if alive.is_empty()
    || alive.iter().any(|&i| beats(&scored[i].0, &scored[0].0, gate_z))
{
    break;
}
```

Measured on 2026-09-28–29 against `cf01da8`, using the same release profile
and local `target-cpu=native` flag on a Ryzen 7 8700F.  Serial Criterion
runs used 100 measurements per case, pinned to CPU 2, baseline followed
by prototype:

| Decision | Before | Early acceptance | Time reduction |
| -------- | -----: | ---------------: | -------------: |
| `mc:128` | 7.050 ms | 5.569 ms | 21.0% |
| `mc:512` | 29.550 ms | 18.491 ms | 37.4% |

Both fixture improvements had `p < 0.05`.  All worlds are sampled before
scoring, so only rollout work is saved.  This is one discard fixture,
not average whole-game cost.

The whole-game diagnostic used `mc:512`, 500 mirrored pairs **per seed**
on seeds 7 and 8 (2000 games per arm/opponent), exact EAAI rules, and
scored-hand-only dealer alternation.  Each arena ran separately with
eight trial-level Rayon threads pinned to CPUs 0–7, without the
`parallel` feature.  Prototype legs ran before baseline legs.  Intervals
below use mirrored pairs as clusters; scores are raw points per game:

| Opponent | Stopping rule | Game wins (95% CI) | Scores, bot–opponent | Exact sweep p | Games/s |
| -------- | ------------- | ----------------: | ------------------: | ------------: | ------: |
| EAAI | Existing | 74.65% (72.91–76.39%) | 98.05–61.47 | < .001 | 4.865 |
| EAAI | Early acceptance | 71.75% (69.92–73.58%) | 96.35–63.87 | < .001 | 5.582 |
| MARJJ v5 surrogate | Existing | 55.15% (53.18–57.12%) | 85.03–78.97 | < .001 | 5.406 |
| MARJJ v5 surrogate | Early acceptance | 51.55% (49.50–53.60%) | 82.52–81.96 | .152 | 6.138 |

MARJJ here is the benchmark-only **host-engine adaptation** of the public
v5 source, which is not established as the submitted championship build;
these are not original-agent tournament reproductions.  The unchanged
adapter uses the validated pinned [conformance receipt](../contrib/strong-conformance/receipt.json).
The [full provenance qualifications](strong-opponents.md) still apply.

Observed win share fell on both seeds: EAAI by 3.6 and 2.2 percentage
points, and MARJJ by 4.7 and 2.5.  Pooled raw score margin fell from
36.58 to 32.48 against EAAI and from 6.06 to 0.56 against MARJJ.
Dead hands rose from 26 to 33 and from 480 to 499, respectively; those
hands were redealt within the measured games.  Whole-game throughput
rose only 14.7% and 13.5%, much less than the hard-discard speedup.

The exact p-values test each arm against its opponent, **not** the
before/after difference.  These summaries do not retain cross-revision
pair covariance, so no paired significance claim about that difference
follows.  Revisions reuse seeded shuffle streams, but changed play can
change later dealer assignments and deals.  This diagnostic does not
replace either fixed publication panel.

Reject this first-crossing rule as a default optimization: the observed
strength tradeoff does not justify its speed gain.  Clearing the gate
against the incumbent also need not identify the best challenger; the
existing `elimination_matches_the_full_read` fixture changes its pick
under the prototype.  All three release strength tripwires nevertheless
passed (including 728/1000 EAAI game wins), illustrating why those loose
floors alone are insufficient.  Production code, defaults, and the
original regression tests remain unchanged.

The [retained evidence](early-acceptance.json) includes the exact patch,
both source hashes, all four `gin-rummy-arena/v1` reports, Criterion
estimates, and prototype test logs.  To reproduce, build `cf01da8`, save
the baseline, then apply the retained patch and repeat the same commands:

```console
taskset -c 2 cargo bench --bench decision -- 'monte carlo turn, (128|512) samples' --save-baseline early-before
# With the prototype applied, use --baseline early-before instead.
cargo test --release --test strength -- --ignored --nocapture
RAYON_NUM_THREADS=8 taskset -c 0-7 cargo run --release --example arena -- --games 500 --p1 mc:512 --p2 eaai --rules eaai --alternate-dealer --seeds 7,8 --format json
# Repeat the arena command with --p2 marjj-v5-surrogate on both revisions.
```

## 6. Allocations (profiled; fixed buffers deferred)

Every candidate/world evaluation clones the stock and the pile into two
`Vec`s in `MonteCarloBot::sim`.  The pile can also reallocate on discard,
and knock settlement allocates its meld spread.  Replacing the stock and
pile with `[Card; 52]` plus lengths would remove some of these calls, but
the measured allocator cost does not justify that change yet.

Profiled on 2026-09-29 at `c0f3ccd`, with the release profile and local
`target-cpu=native`, on a Ryzen 7 8700F pinned to CPU 2.  The existing
serial Criterion discard fixture ran with `--profile-time 30`, once per
budget, 512 before 128.  Gperftools `libprofiler` 2.18.1 requested 1000 Hz
CPU sampling; the glibc allocator was unchanged.  Profiles include
Criterion warmup and process setup.

| Decision | CPU samples | Samples in allocator stacks | Allocator share | Solver self share |
| -------- | ----------: | --------------------------: | --------------: | ----------------: |
| `mc:128` | 25,034 | 169 | 0.68% | 61.1% |
| `mc:512` | 30,942 | 209 | 0.68% | 63.0% |

Allocator share counts each sampled stack containing a glibc allocation,
reallocation, or free routine once, including its callees.  It covers
**all** allocations in the process, not just the stock and pile.  Solver
self share sums `search`, `deadwood`, and `best_melds`, excluding their
callees.  These shares are CPU samples, not allocation counts or a
predicted fixed-buffer speedup: inlined bookkeeping, copies outside
allocator calls, and cache effects are not isolated.

The allocator is not a leading cost in this fixture.  Defer fixed buffers
until a representative workload puts allocation among the hot frames.
This is not a whole-game or parallel-allocation profile; production code
and decisions remain unchanged, so no strength panel was rerun.

The [retained reports](allocation-profile.json) contain symbolized sample
counts, tool versions, executable hash, and exact commands.  To reproduce,
build the serial benchmark and use the executable path Cargo prints as
`bench` below.  Adjust the library path for the local gperftools install;
use `libprofiler`, which does not replace the allocator.

```console
cargo bench --bench decision --no-run
bench='target/release/deps/decision-<hash>'
taskset -c 2 env LD_PRELOAD=/usr/lib64/libprofiler.so.0 CPUPROFILE=/tmp/gin-alloc-512.prof CPUPROFILE_FREQUENCY=1000 "$bench" --bench 'monte carlo turn, 512 samples' --profile-time 30
# Repeat with 128 in both the profile filename and benchmark filter.
curl -fsSL https://raw.githubusercontent.com/gperftools/gperftools/gperftools-2.10/src/pprof -o /tmp/gin-alloc-pprof
perl /tmp/gin-alloc-pprof --text "$bench" /tmp/gin-alloc-512.prof
perl /tmp/gin-alloc-pprof --text --focus='(__.*(malloc|free|realloc)|_int_(malloc|free|realloc)|unlink_chunk)' "$bench" /tmp/gin-alloc-512.prof
```

## Skipped

A per-decision solver memo keyed by the 64-bit hand bits.  Hit rates are
unknown; add a debug counter first and build the memo only if hits are
common.
