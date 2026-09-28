# Performance backlog

Ideas for making `MonteCarloBot` decide faster, ranked by expected payoff
per line of diff.  A Monte Carlo turn is pure rollout cost: the saved
Criterion runs put a greedy self-play round at about 0.06 ms and an
`mc:512` discard decision at about 56 ms, and each rollout turn performs
roughly three deadwood solves (`improves`, `best_melds`, and the knock
re-solve).  Everything below shrinks either the number of solves or the
cost of one.

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
| default release profile | 8.05 ms |
| `-C target-cpu=native`, `lto = "fat"`, `codegen-units = 1` | 6.85 ms |

Put `lto = "fat"` and `codegen-units = 1` in `[profile.release]`.  Keep
`target-cpu=native` in a local `.cargo/config.toml` only, so crates.io
users get a portable binary.  Not yet applied.

## 2. Delete redundant solves in the rollout (pure reuse)

All in `src/sim.rs`, `Sim::rollout_observed`:

- `improves` (`src/heuristic.rs`) already solves the eleven-card hand.
  When it returns true, the next `Shed` phase solves the identical hand
  again.  Return the `Melds` from `improves` and carry them into the shed.
- `improves` calls `deadwood(hand)` on the ten-card hand when the pile
  top ends up melded.  That number is exactly the `rest` the same seat
  computed at its previous shed.  Cache one `u8` per seat in `Sim`.
- `Sim::knock` re-solves `hand - card` after `best_shed`.  When the shed
  card was unmelded, the arrangement is unchanged and `best_shed` already
  holds it.

Together these remove one to two of the three solves per rollout turn.

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
