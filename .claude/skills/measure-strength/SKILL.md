---
name: measure-strength
description: Statistically evaluate whether a change to HeuristicBot, MonteCarloBot, their defaults, or the shared greedy core made the bots stronger or weaker. Use after any change to decision logic, sampling, or tuning knobs, and before claiming any strength number in docs or commit messages.
---

# Measure bot strength without fooling yourself

Strength differences in this crate are a few percentage points; eyeballing
a handful of games proves nothing.  Everything below runs in **release
mode** — debug builds are far too slow for Monte Carlo rollouts and any
timing you observe in them is meaningless.

## Baselines (default configs, `Rules::default()`)

- `mc:128` beats the default `greedy` in ≈64% of decisive rounds (63.9%,
  62.3–65.3, over 2000 mirrored pairs at seed 1), at ~10 ms per average
  serial turn (a hard first discard runs the full budget for ~25 ms; the
  `parallel` feature divides either by most of the cores).  The default
  heuristic is tuned for whole-game play and so concedes single rounds;
  this round figure is not a game-strength number.
- The tripwire (`tests/strength.rs`) holds three floors, each ~4σ below
  the measured rate so that only an accident trips it: >52.5% of 1000
  rounds against `greedy` (a true 65% bot passes with near certainty, an
  even bot sneaks through less than 6% of the time), >56% of 4000 games
  and >53% of 1000 games against the `eaai` baseline for `greedy` and the
  default `MonteCarloBot` respectively — those two fixtures realize 59.6%
  and 59.5%, matching the arena measurements below.  The game floors are
  what guard score-aware play; the round floor cannot see it, since both
  scores stay level inside a single round.  Whole minutes: budget ~4 for
  the Monte Carlo game leg even fanned across the cores.
- Against the `eaai` challenge baseline (`--rules eaai
  --alternate-dealer`, mirrored pairs, games pooled over seeds 7 and 8):
  `greedy` wins 39.4% of rounds yet 59.7% of 12 000 games — gin-hunting
  concedes rounds and banks matches, so quote both; `mc:64` wins 51.5%
  of rounds and 54.8% of 12 000 games (still below greedy over games);
  `mc:128` wins 52.7% of rounds and 59.6% of 8000 games, closing the
  gap.  `mc:64` beats `greedy` head-to-head over whole games (53.0% of
  12 000) — exploitation of the weak baseline is not transitive with
  head-to-head strength.  Published EAAI-21 entries report ≈55–68%
  against the same baseline (metrics vary by paper), the cross-engine
  calibration these numbers exist for.

If a change moves these baselines, update them here, in `tests/strength.rs`,
in the doc comment on `MonteCarloBot::samples`, and in README's benchmark
table — which `scripts/bench-panel.sh` regenerates (see below), so never
hand-edit a cell of it.

## Procedure

1. Regression gate:

   ```console
   cargo test --release --test strength -- --ignored
   ```

2. Head-to-head measurement with the arena.  Trials are seeded by index
   and fan out across the CPUs; by default each trial is a *mirrored
   pair* — both bots play the same deal(s) from both seats — and the
   run ends with a significance line testing the paired difference
   (`--unpaired` reverts to independent trials and a sign test):

   ```console
   cargo run --release --example arena -- --rounds 4000 --p1 greedy --p2 mc:64 --seed 7
   cargo run --release --example arena -- --games 3000 --p1 mc:64 --p2 eaai --rules eaai --alternate-dealer --seed 7
   ```

   Use `--alternate-dealer` for any number quoted against the EAAI
   literature: the challenge alternated the deal every hand, where
   gin-rummy's `Game` rotates it to each round's winner.  The two
   protocols measurably shift game rates (a few points), so never mix
   them in one comparison.

3. To compare old versus new code, run the *same* command (same `--seed`,
   same count) on both revisions and compare.  The deal stream depends
   only on `--seed` and the trial index, so the two revisions play the
   same deals and the comparison is itself paired.

4. Never build the arena or tune with `--features parallel`: in-decision
   parallelism would fight the trial-level fan-out for the same rayon
   pool.

5. Publish the panel.  Once a change is believed and merged, regenerate
   README's table rather than editing it:

   ```console
   scripts/bench-panel.sh > panel.md   # ~1.5 hours, arena log on stderr
   ```

   The script pins the bots, seeds and counts of the published panel, so
   at an unchanged commit it reprints the same table — a rerun that
   differs means the numbers moved, not that the measurement wandered.
   Shrink it for a dry run: `ROUND_PAIRS=20 GAME_PAIRS=20
   GAME_PAIRS_128=20 scripts/bench-panel.sh`.

## Reading the numbers

- The arena prints 95% Wilson score intervals per bot and a paired
  z/p line for the head-to-head difference.  Trust the p line over
  eyeballing interval overlap — pairing makes it strictly sharper.
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
- Compare within one rules preset; strength does not transfer across
  `--rules modern|classic|palace`.

## Score-aware changes are game-only

Anything that reads the game score from the `View` — the heuristic's
`score_awareness` knob and `MonteCarloBot`'s `game_value` objective — can
only differentiate itself over **whole games**; a single round carries a
level scoreboard.  The heuristic's shift is exactly inert at a zero
margin.  `GameValue::Table`, the Monte Carlo default, is only *nearly*
inert: its value function is locally linear at level scores with the
empirically measured slope, so it reproduces round-point play until the
board goes lopsided.  That is measured, not assumed — `mc:64` wins 51.5%
of decisive rounds against the baseline under either value function, at
9.22 against 9.21 points per round, while over whole games the same
change is worth +2.1 and +2.7 points on two seeds.  The round tripwire
and `arena --rounds` therefore neither catch a regression here nor credit
an improvement.

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
  --p1 mc:64 --p2 mca:64 --rules eaai --alternate-dealer --seed 7
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
