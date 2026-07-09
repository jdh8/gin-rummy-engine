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
  per turn.  The default heuristic is tuned for whole-game play and so
  concedes single rounds; this round figure is not a game-strength number.
- The tripwire (`tests/strength.rs`) demands >52.5% over 1000 rounds: a
  true 65% bot passes with near certainty, an even bot sneaks through
  less than 6% of the time.

If a change moves these baselines, update them here, in `tests/strength.rs`,
and in the doc comment on `MonteCarloBot::samples`.

## Procedure

1. Regression gate:

   ```console
   cargo test --release --test strength -- --ignored
   ```

2. Head-to-head measurement with the arena (seats and the dealer alternate
   every trial, so there is no first-move bias to correct for):

   ```console
   cargo run --release --example arena -- --rounds 4000 --p1 greedy --p2 mc:64 --seed 7
   cargo run --release --example arena -- --games 200 --p1 mc:16 --p2 mc:64 --seed 7
   ```

3. To compare old versus new code, run the *same* command (same `--seed`,
   same `--rounds`) on both revisions and compare the printed intervals.

## Reading the numbers

- The arena prints 95% Wilson score intervals.  Rough half-widths near a
  55% win rate: 1000 rounds → ±3 points, 4000 → ±1.5, 10 000 → ±1.
- If the two candidates' intervals overlap heavily, the run is
  inconclusive.  Increase `--rounds`; do **not** re-roll seeds until one
  looks better — that is p-hacking and the "improvement" will not
  replicate.
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
```

Criterion benches per-decision latency for the heuristic and for
`mc:16`/`mc:64`.  A strength win that triples decision time is a loss for
interactive use; report both.

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
