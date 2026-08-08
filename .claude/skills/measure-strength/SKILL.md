---
name: measure-strength
description: Statistically evaluate whether a change to HeuristicBot, MonteCarloBot, their defaults, or the shared greedy core made the bots stronger or weaker. Use after any change to decision logic, sampling, or tuning knobs, and before claiming any strength number in docs or commit messages.
---

# Measure bot strength without fooling yourself

Strength differences in this crate are a few percentage points; eyeballing
a handful of games proves nothing.  Everything below runs in **release
mode** — debug builds are far too slow for Monte Carlo rollouts and any
timing you observe in them is meaningless.

## Baseline status

- Under `Rules::default()`, the standing smoke observation is that
  `mc:128` beats the default `greedy` in roughly 64% of decisive rounds
  and costs about 10 ms per average serial turn.  Its old 63.9%
  (62.3–65.3) summary used the former per-play interval analysis; retain
  it for regression orientation, not as a publishable interval.  A new
  claim needs a pair-cluster interval from the current arena.
- The ignored tripwire (`tests/strength.rs`) holds deliberately loose
  floors for `mc:128` versus `greedy` rounds and for `greedy` and default
  Monte Carlo versus `eaai` games.  A floor catches a large accident; it
  is neither an estimator nor publication evidence.  The EAAI fixtures
  also need rebaselining after the dealer-protocol correction, so do not
  quote their historical realized rates.
- Every EAAI-baseline rate currently printed in README is historical.
  Those runs predate both the exact `eaai_rules()` preset and the audit
  showing that a dead hand retains the dealer.  The prior panel reported
  39.4%/59.7% for `greedy`, 51.5%/54.8% for `mc:64`, and 52.7%/59.6%
  for `mc:128` (round/game where available), but none is a current
  corrected-protocol claim.  Regenerate the complete fixed panel before
  comparing it with EAAI literature or a strong external opponent.

When the corrected panel is complete, update this file,
`tests/strength.rs`, the relevant `MonteCarloBot` docs, README, and the
strong-opponent report together.  `scripts/bench-panel.sh` owns the README
table cells, so never hand-edit an individual result.

## Procedure

1. Regression gate:

   ```console
   cargo test --release --test strength -- --ignored
   ```

2. Head-to-head measurement with the arena.  Trials are seeded by index
   and fan out across the CPUs.  By default each trial is a *mirrored
   pair*: the bots swap seats under common random numbers.  A round pair
   clones one exact deal; a game pair starts both orientations from the
   same seeded shuffle stream.  If one orientation scores where the other
   goes dead, the corrected dealer rule makes their later dealer sequences
   diverge.  This remains a valid paired trial, but it is not an exact
   replay of every later game state (`--unpaired` plays one orientation):

   ```console
   cargo run --release --example arena -- --rounds 4000 \
     --p1 greedy --p2 mc:64 --seeds 7,8 --format json
   cargo run --release --example arena -- --games 3000 \
     --p1 mc:128 --p2 eaai --rules eaai --alternate-dealer \
     --seeds 7,8 --format json
   ```

   Use `--alternate-dealer` for any number quoted against the EAAI
   literature.  It selects
   `DealerRotation::AlternateAfterScoredRound`: a scored hand flips the
   dealer and a dead hand is redealt by the same dealer.  Pair it with
   `--rules eaai`, the public `eaai_rules()` preset with no Big Gin, box,
   game, or shutout bonus.  The default protocol is winner-deals-next, so
   never mix the two in one comparison.

   `--seeds 7,8` emits per-seed and pooled estimates in one run.
   `--format json` emits schema `gin-rummy-arena/v1`, including raw score,
   finish attribution, sufficient cluster moments, primary test fields,
   and reproducibility metadata.  Bare `mc` and `mca` mean 128 samples;
   spell out `mc:128` or `mca:128` in a published command.

3. To compare old versus new code, run the *same* command (same explicit
   `--seeds` and count) on both revisions and compare.  The shuffle stream
   depends only on the seed and trial index, so the revisions use common
   random numbers.  Policy or dead/scored differences can change later
   dealer assignments and prevent an exact whole-game replay; report that
   honestly rather than claiming identical later deals.

4. Never build the arena or tune with `--features parallel`: in-decision
   parallelism would fight the trial-level fan-out for the same rayon
   pool.

5. Publish the panel.  Once a change is believed and merged, regenerate
   README's table rather than editing it:

   ```console
   scripts/bench-panel.sh > panel.md   # ~1.5 hours, arena log on stderr
   ```

   The checked-in README panel predates the corrected protocol and remains
   historical until this full run replaces it.  The script pins the bots,
   seeds and counts, so at an unchanged commit it reprints the same table
   — a rerun that differs means the numbers moved, not that the measurement
   wandered.
   Shrink it for a dry run: `ROUND_PAIRS=20 GAME_PAIRS=20
   GAME_PAIRS_128=20 scripts/bench-panel.sh`.

## Strong-opponent panel

Use the benchmark-only `gold-paper` host adaptation and
`marjj-v5-surrogate` only with their provenance qualifications.  Gold's
upstream policy solves meld decomposition exactly; it is not a
game-theoretically optimal full-game player.  The public MARJJ repository
is associated with the 2021 challenge winner, but the separately named v5
source is not established as the submitted championship build.  Run the
opt-in source-conformance checks described by
`scripts/check-strong-conformance.sh` before publishing either adaptation.

Keep commands, source identities, exclusions, JSON evidence, and results in
[`docs/strong-opponents.md`](../../../docs/strong-opponents.md).  Its table
must stay pending until corrected-protocol runs actually complete; upstream
claims about those opponents are context, never substitute measurements of
this engine.

## Reading the numbers

- For mirrored runs, headline each bot's 95% **pair-cluster** confidence
  interval.  The pair, not each orientation, is the independent sampling
  unit.  Wilson intervals are a fallback for unpaired runs or too few
  clusters, never the mirrored-run headline.
- The primary paired hypothesis test is the exact two-sided sign test over
  *sweeps*: pairs in which one bot won both seat orientations.  Split pairs
  carry no sign; if neither bot sweeps a pair, the exact p-value is 1.  Use
  `comparison.primary_test` and `primary_p_value` from JSON.  The paired
  normal z/p fields are diagnostics, not headline evidence.
- Power, unpaired rule of thumb: detecting a 4-point game-rate gap at
  80% power needs ≈2 400 games per arm; a 2-point gap ≈9 700.  Pairing
  reduces these; budget 3 000 pairs for headline claims and do not
  chase sub-2-point differences.
- If a run is inconclusive, increase the count; do **not** re-roll seeds
  until one looks better — that is p-hacking and the "improvement" will
  not replicate.  Confirm any winner on a second seed before believing
  it.
- Dead hands are excluded from decisive rounds, so a change that raises
  the dead-hand rate can "improve" the decisive win rate while scoring
  fewer points.  Check the results line (knocks/undercuts/gins/dead) and
  points per round, not just the percentage.
- Compare within one rules preset and dealer rotation; strength does not
  transfer across `--rules modern|classic|palace|eaai` or between
  winner-deals and EAAI scored-hand alternation.

## Score-aware changes are game-only

Anything that reads the game score from the `View` — the heuristic's
`score_awareness` knob and `MonteCarloBot`'s `game_value` objective — can
only differentiate itself over **whole games**; a single round carries a
level scoreboard.  The heuristic's shift is exactly inert at a zero
margin.  `GameValue::Table`, the Monte Carlo default, is only *nearly*
inert: its value function is locally linear at level scores with the
empirically measured slope, so it reproduces round-point play until the
board goes lopsided.  Its table is selected by both `Rules` and
`DealerRotation`; a value function solved for winner-deals is not evidence
for the EAAI protocol.  Historical pre-correction measurements found the
two value functions indistinguishable in rounds but separated in whole
games.  Regenerate those rates under the corrected protocol before quoting
their former 51.5% round rate, point totals, or +2.1/+2.7-point game lifts.
The round tripwire and `arena --rounds` therefore neither catch a
score-aware regression nor credit an improvement.

That is also the bar a *new* value function has to clear.  An earlier
shaped equity — a win-probability race over the points still needed —
measured weaker over whole games (−2 points over 4000 palace games,
nothing gained elsewhere) because it bent mid-game play at level scores
where the round-point objective was already right.  `GameValue::Table`
beat it by being faithful exactly there and deviating only where the game
recursion says a point's worth genuinely changes.  Put two value
functions head to head over paired games; `mca` is the arena's affine
arm, kept so this comparison stays reproducible:

```console
cargo run --release --example arena -- --games 3000 \
  --p1 mc:64 --p2 mca:64 --rules eaai --alternate-dealer \
  --seeds 7,8 --format json
```

Measure over games, never rounds:

```console
cargo run --release --example tune -- --games 20000 --seed 1 \
  --knock 4 --awareness 0,32 --opponent mc:64
```

`tune` pits a candidate `HeuristicConfig` against a fixed opponent (`greedy`,
`greedy:knock:awareness`, or `mc:N`) over whole games with paired seeds, and
sweeps a grid of `(knock_threshold, score_awareness)`.  The round-based
tripwire and `arena --rounds` cannot see these changes — they neither catch
a regression nor credit an improvement.  Always confirm the winner against a
**strong** opponent (`--opponent mc:64`), not just the default greedy: a
config that beats weak greedy but not `mc` is exploiting it, not genuinely
stronger.  Search on one seed, re-confirm the single best arm on another.

## Speed

```console
cargo bench
cargo bench --features parallel
```

Criterion benches per-decision latency for the heuristic and for
`mc:16`/`64`/`128` plus an Expert-sized `mc:1024` arm; the second line
gives the multi-core numbers.  The bench position is a deliberately hard
first discard — every shed stays plausible, so candidate elimination
saves little there and whole-game throughput (the arena's rounds/s) is
the fairer average-cost read.  A strength win that triples decision time
is a loss for interactive use; report both.

## The statistics inside MonteCarloBot

The bot deviates from the greedy incumbent only when the paired advantage
clears two standard errors (`beats` in `src/mc.rs`); common random numbers
(the same sampled worlds for every candidate) make the pairing work.
Worlds are rolled in growing batches, and a challenger the incumbent beats
by that same bar is dropped at a batch boundary — the decision stops when
none remain — so easy decisions cost a fraction of the sample budget while
survivors keep unbatched-identical statistics.  If you touch the sampling,
`beats`, or the batching, re-run this whole procedure — loosening the gate
usually *weakens* the bot, because deviating on noise plays worse than the
baseline.
