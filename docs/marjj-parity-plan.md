# Reaching strength parity with MARJJ

Written 2026-08-09 against the completed fixed strong-opponent panel
([report](strong-opponents.md), [raw JSON](strong-opponents.json)).
The companion [design document](marjj-parity-design.md) carries the
technical designs this plan schedules.

Throughout, "MARJJ" means `marjj-v5-surrogate`: the host-engine
reconstruction of the later public MARJJ v5 file, measured under the
corrected EAAI protocol.  The surrogate is associated with the 2021
challenge winner but is not established as the championship binary; the
target of this plan is the surrogate itself, which is the strongest
opponent this engine can actually be measured against.

## 1. Goal

Bring the **default-configuration** `mc:128` to statistical parity with
`marjj-v5-surrogate` over whole games, without giving back strength
anywhere else.  A bespoke anti-MARJJ build would be overfitting theater;
the deliverable is a shipped default that handles gin-campers because it
understands opponents, not because it memorized this one.

Operational definition, using the panel's own protocol (`--rules eaai
--alternate-dealer`, mirrored pairs, seeds 7 and 8, 3000 game pairs per
seed, pair-cluster intervals, exact pair-sweep sign test):

- **Parity reached**: pooled game win share ≥ 50% with the 95%
  pair-cluster lower bound ≥ 48%, both seeds agreeing in direction.
- **Stretch**: lower bound ≥ 50% — strictly stronger, not merely level.

Guards that must hold in the same measurement cycle before any default
changes (all are non-inferiority bounds, two points under the current
panel figures):

- vs `EaaiSimpleBot`, games: `mc:128` ≥ 57.9% (now 59.9%).
- vs `gold-paper`, games: `mc:128` ≥ 65.1% (now 67.1%).
- `tests/strength.rs` tripwire floors keep passing untouched until the
  final panel justifies raising them.
- Mean serial decision latency at defaults stays within ~2× the current
  ~10 ms average turn (`cargo bench`); analysis configs may exceed it,
  the interactive default may not.

## 2. Where we stand

From the fixed panel, candidate game win share against MARJJ:

| Candidate | Games won | Points/round | K/U/G (candidate) | K/U/G (MARJJ) |
|-----------|----------:|-------------:|------------------:|--------------:|
| `greedy`  | 29.2% (28.4–30.0%) | 7.87 vs 14.43 | 6678/32/1850 | 1310/2627/3402 |
| `mc:64`   | 30.2% (29.4–31.0%) | 9.15 vs 13.47 | 8358/16/760  | 1361/4007/1400 |
| `mc:128`  | 34.2% (33.4–35.0%) | 9.72 vs 13.45 | 8131/16/1002 | 1393/3758/1556 |

The paradox that defines the problem: `mc:128` wins **57.7% of decisive
rounds** yet only 34.2% of games.  Its wins are small knock margins; its
losses are 25-point undercuts and gins.

## 3. Why we lose

MARJJ v5's policy, from the surrogate: take the upcard only when it
lands in a meld and lowers deadwood; discard by minimizing `deadwood +
18 · 0.9^(turn−1) · one-ply-draw-lookahead + danger`; **knock
voluntarily only on turns 1–3**, then play exclusively for gin; choose
the knock spread that minimizes the opponent's layoff.

Our Monte Carlo rollouts model the opposite opponent.  Both rollout
seats knock at the first legal chance (`rollout_knock_self` and
`rollout_knock_opponent` default to `u8::MAX`), and sampled opponent
hands are the best of `pile_len / 2` uniform draws — far weaker, deep
into a round, than a camper who has been assembling melds for ten
turns.  Two systematic errors follow:

1. **Undercuts look impossible.**  In the sampled worlds the opponent's
   deadwood is high, so knocking at 8–10 rates as safe.  In reality
   MARJJ is sitting on a near-gin hand: 3758 of our 11 889 knock
   attempts — **32%** — were undercut at −25 and change apiece.
2. **Gin-hunting looks suicidal.**  The modeled opponent knocks next
   turn in every rollout, so patient lines score terribly and the bot
   knocks instead.  Against an opponent who will never knock, holding
   out is exactly right — MARJJ out-gins `mc:128` 1556 to 1002, and
   out-gins even our gin-leaning `greedy` 3402 to 1850.

Both errors share one root: the rollout's opponent model — policy and
sampled hand strength — does not match the observed opponent.  That is
the thing to fix; everything else is a corollary.

## 4. The path

Phases are ordered by evidence-per-line-of-code: the first experiments
need no library changes at all, because the levers already exist —
`rollout_knock_opponent: 0` *is* a gin-camper model, and
`opponent_strength_percent` already scales hand plausibility.  Each
phase ends with a measured go/no-go against the surrogate (search on
seed 7, confirm on seed 8, per the `measure-strength` skill).

| Phase | Change | Code touched | Gate (win share vs MARJJ) |
|-------|--------|--------------|---------------------------|
| M0 | Point the sweep harness at the surrogate | `examples/tune.rs` only | Smoke run reproduces ~34% at defaults |
| M1 | Sweep existing `McConfig` knobs | none | Best fixed config ≥ ~42%, else re-diagnose |
| M2 | Calibrated opponent-hand sampling | `src/mc.rs` (+ baked curves) | Cumulative ≥ ~46% |
| M3 | Adaptive opponent-archetype inference (opt-in knob) | `src/mc.rs`, driver hook | ≥ 50%, lower bound ≥ 48%, guards pass |
| M4 | Flip the measured default; publish | defaults, panels, docs | Full `bench-strong.sh` + `bench-panel.sh` reruns |

**M0 — harness (design §D1).**  `tune` gains `--opponent marjj`; the
surrogate stays frozen and outside the library API.  Half a day.

**M1 — the free experiments (design §D2).**  Sweep, against MARJJ over
paired whole games: `--opp-knock {255, 10, 0}`, `--rollout-knock
{255, 2, 0}`, `--opp-model {eager, meld}`, `--opp-strength
{100, 200, 400}`, `--max-candidates {4, 6, 8}`; secondarily a `greedy`
lane over `(knock_threshold, score_awareness)`.  Hypothesis: camper
knock model plus stronger sampled hands recovers a large slice of the
gap by itself.  Compute-bound (a day or two of wall clock), zero risk,
and it localizes how much gap remains for real designs.

**M2 — calibrated sampling (design §D3).**  Replace best-of-k-uniform
with sampling toward baked deadwood-by-turn curves measured from
archetype self-play, so a sampled world's opponent is as developed as a
real one at that point in the round.  This is what prices undercut risk
correctly, and the same instrumentation yields the knock-hazard curves
M3 needs.

**M3 — adaptive archetype inference (design §D4, §D5).**  The bot
tracks, across the rounds of a game, when the opponent knocks and how
they draw; classifies them over a small archetype set (eager / balanced
/ camper); and samples worlds from the posterior mixture with each
archetype's rollout policy and hand calibration.  Ships behind a new
`McConfig` knob, default off, so the measurement compares like with
like.  A fresh game starts at today's behavior and converges within a
round or two of evidence — campers reveal themselves fast.

**M4 — ship it.**  Flip the knob's default in its own measured step
(a `McConfig` default change is a strength change by house rule),
re-run both fixed panels, regenerate the README tables from the
scripts, update `docs/strong-opponents.md`, CHANGELOG, and the
tripwire floors, then release per the `release` skill.

## 5. Measurement discipline

Everything in the `measure-strength` skill applies unchanged; the
specific commitments for this effort:

- Sweeps run through `tune` (paired seeds, whole games); claims run
  through `arena` / `bench-strong.sh` at panel counts.  No optional
  stopping, no seed shopping; an inconclusive run gets more pairs, not
  a new seed.
- Every phase measures the same three-opponent matrix before merging:
  MARJJ (the target), `EaaiSimpleBot` and `gold-paper` (the guards).
  A config that beats MARJJ by donating strength elsewhere fails its
  gate.
- Points per round and the K/U/G finish mix get read alongside win
  share — the undercut count is this plan's most sensitive dial.
- Latency via `cargo bench` whenever sampling or candidate logic
  changes.

## 6. Already measured — do not redo

Institutional memory from CHANGELOG and the `mc.rs`/skill docs; these
cost real compute and are settled unless the surrounding design
changes:

- `rollout_knock_self: 2` beats the EAAI baseline but loses to an
  unmodified `mc:64` (47.9% vs 48.9%) — an exploit of knock-ASAP
  opponents, not general strength.  Middling self thresholds 4 and 8
  are worse than either extreme.
- `OpponentModel::MeldOnly` alone was no better than `Eager` against
  the baseline it models (53.9% vs 54.8%) — belief fidelity is not
  monotone in strength.
- The cold-card sampling penalty measured flat (+0.2/−0.2 points).
- A shaped win-probability-race equity measured *weaker* than affine
  over whole games; `GameValue::Table` won by being faithful at level
  scores.  Do not bend the value function to this problem.
- Loosening `gate_z` usually weakens the bot — deviating on noise
  plays worse than the greedy baseline.  The fix for wrong deviations
  is world realism, not a looser gate.

## 7. Risks

- **Overfitting the surrogate.**  MARJJ v5 has faithful bugs (a stale
  opponent mask, a danger-score typo).  Mitigation: model *archetypes*
  (knock timing, draw rule, hand development), never the surrogate's
  literal scoring; hold the guard matrix; remember the provenance
  caveat — the surrogate may not be the champion build.
- **Inference starves in short games.**  A 100-point EAAI game is only
  ~5–10 rounds.  Mitigation: in-round signals (knock silence over
  turns, upcard behavior) via baked hazard curves, a prior that starts
  at today's default behavior, and hysteresis so one odd round cannot
  flip the model.
- **Invariant drag.**  The rollout policy doubles as `Sim`'s policy and
  the value tables are measured through `Sim` (invariants 4–5).  None
  of M1–M3 changes mechanics, so the equivalence proptest and baked
  value models stay valid; the design doc carries the full analysis.
- **Compute.**  A full strong panel is hours; sweeps are days of
  background wall clock.  Budgeted per phase above; nothing here needs
  new hardware.

## 8. Non-goals and the fallback ladder

No neural networks, no CFR, no reinforcement learning in this plan.
The gap has a specific, diagnosed cause — a mismatched opponent model
inside an otherwise sound equity search — and the remedies above are
deterministic, testable, and cheap.  If M3 stalls below ~45% despite
calibrated beliefs, the next rungs in order are: per-card posterior
sampling weighted by observed discards (a true particle filter over
opponent hands), then offline-tuned rollout policies per archetype.
Learned evaluation is the last resort and out of scope until the
search-and-modeling ladder is exhausted.
