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

- `mc:128` beats the default `greedy` in ≈65% of decisive rounds, at ~10 ms
  per average serial turn (a hard first discard runs the full budget for
  ~25 ms; the `parallel` feature divides either by most of the cores).  The
  default heuristic is tuned for whole-game play and so concedes single
  rounds; this round figure is not a game-strength number.
- The tripwire (`tests/strength.rs`) demands >52.5% over 1000 rounds: a
  true 65% bot passes with near certainty, an even bot sneaks through
  less than 6% of the time.
- Against the `eaai` challenge baseline (`--rules eaai
  --alternate-dealer`, mirrored pairs, games pooled over seeds 7 and 8):
  `greedy` wins 39.4% of rounds yet 59.7% of 12 000 games — gin-hunting
  concedes rounds and banks matches, so quote both; `mc:64` wins 51.5%
  of rounds and 54.0% of 12 000 games (genuinely below greedy, z ≈ 6 per
  seed); `mc:128` wins 52.8% of rounds and 59.4% of 8000 games, closing
  the gap.  `mc:64` beats `greedy` head-to-head over whole games (52.9%
  of 6000) — exploitation of the weak baseline is not transitive with
  head-to-head strength.  Published EAAI-21 entries report ≈55–68%
  against the same baseline (metrics vary by paper), the cross-engine
  calibration these numbers exist for.

If a change moves these baselines, update them here, in `tests/strength.rs`,
and in the doc comment on `MonteCarloBot::samples`.

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
`score_awareness` knob and `MonteCarloBot`'s game equity objective — can
only differentiate itself over **whole games**; a single round carries a
level scoreboard.  The heuristic's shift is exactly inert at a zero
margin; the Monte Carlo equity stays affine in round points until a
rollout can end the game, so it too is inert in standalone rounds (a
100-point clinch from a level board is the vanishing exception).  A
shaped equity that also bent mid-game play — a win-probability race over
the points still needed — measured *weaker* over whole games (−2 points
over 4000 palace games, nothing gained elsewhere); don't reintroduce one
without beating that bar.  Measure over games, never rounds:

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
