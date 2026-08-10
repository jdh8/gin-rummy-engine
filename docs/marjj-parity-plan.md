# Reaching strength parity with MARJJ

Written 2026-08-09 against the completed fixed strong-opponent panel
([report](strong-opponents.md), [raw JSON](strong-opponents.json)).
The companion [design document](marjj-parity-design.md) carries the
technical designs this plan schedules.  The M2.5 diagnostic evidence is
retained separately as [aggregate JSON](marjj-m2.5-diagnostic.json).

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

- vs `EaaiSimpleBot`, games: `mc:128` ≥ 67.2% (now 69.2%).
- vs `gold-paper`, games: `mc:128` ≥ 72.5% (now 74.5%).
- `tests/strength.rs` keeps the greedy game floor at 55% and raises the
  default Monte Carlo game floor to 60.9%, seven points under its current
  fixed fixture.
- Mean serial decision latency at defaults stays within ~2× the current
  ~10 ms average turn (`cargo bench`); analysis configs may exceed it,
  the interactive default may not.

## 2. Where we stand

From the fixed panel, candidate game win share against MARJJ:

| Candidate | Games won | Points/round | K/U/G (candidate) | K/U/G (MARJJ) |
|-----------|----------:|-------------:|------------------:|--------------:|
| `greedy`  | 29.2% (28.4–30.0%) | 7.87 vs 14.43 | 6678/32/1850 | 1310/2627/3402 |
| `mc:64`   | 42.4% (41.6–43.2%) | 12.60 vs 14.41 | 4219/13/4014 | 1380/1927/3913 |
| `mc:128`  | 46.7% (45.9–47.5%) | 13.48 vs 14.24 | 3670/17/4559 | 1394/1466/4252 |

The patient default has narrowed the problem: `mc:128` wins **53.7% of
decisive rounds** yet 46.7% of games, and trails by only 0.76 points per
round.  It now out-gins MARJJ 4559 to 4252, but MARJJ still undercuts it
1466 times while being undercut only 17 times.

## 3. Original diagnosis — now refuted

The following was the hypothesis that scheduled M1 and M2, not the current
explanation.  Both interventions later contradicted it: modeling a camper's
knock policy hurt in M1, and modeling a developed hand hurt in M2.  M2.5
below records what actually changes when the latter is enabled.

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

The proposed common root was the rollout's mismatched opponent model.  M1,
M2, and M2.5 retain this section as the hypothesis they tested rather than
as a conclusion.

## 4. The path

Phases are ordered by evidence-per-line-of-code: the first experiments
need no library changes at all, because the levers already exist —
`rollout_knock_opponent: 0` *is* a gin-camper model, and
`opponent_strength_percent` already scales hand plausibility.  Each
phase ends with a measured go/no-go against the surrogate (search on
seed 7, confirm on seed 8, per the `measure-strength` skill).

| Phase | Change | Code touched | Gate (win share vs MARJJ) |
|-------|--------|--------------|---------------------------|
| M0 | Point the sweep harness at the surrogate | `examples/tune.rs`, plus the docs it invalidates | Smoke run reproduces ~34% at defaults |
| M1 | Sweep existing `McConfig` knobs | none | Best fixed config ≥ ~42%, else re-diagnose |
| M2 | Calibrated opponent-hand sampling | `src/mc.rs` (+ baked curve) | Cumulative ≥ ~46% — *missed: −1.2 vs MARJJ, ships off* |
| M2.5 | Decompose default vs calibrated play by finish channel | benchmark-only example + aggregate JSON | Stable leading channel across seeds — *met: MARJJ gin losses* |
| M3 | Adaptive opponent-archetype inference (opt-in knob) | `src/mc.rs`, driver hook | *blocked pending a profitable archetype arm and redesign* |
| M4 | Flip the measured default; publish | defaults, panels, docs | Full `bench-strong.sh` + `bench-panel.sh` reruns |

**M0 — harness (design §D1).**  `tune` gains `--opponent
marjj-v5-surrogate` and `--opponent gold-paper`, and — the part §D1
originally missed — `--alternate-dealer`, without which every sweep runs
under winner-deals-next and selects arms against a different Monte Carlo
value table than the panel measures.  The surrogate stays frozen and
outside the library API.  Half a day.

*Gate met.*  At 2000 unpaired games on seed 7 under `--rules eaai
--alternate-dealer`, default `mc:128` won 34.4% (Wilson 32.3–36.5%)
against `marjj-v5-surrogate`, reproducing the panel's 34.2%; the same
command against `eaai` returned 59.0% (56.8–61.1%) against the baseline
panel's 59.9%, which isolates the new dealer loop from the new opponent.
`tune`'s unflagged path is byte-identical to its pre-change output.

**M1 — the free experiments (design §D2).**  Sweep, against MARJJ over
paired whole games under `--rules eaai --alternate-dealer`:
`--opp-knock {255, 10, 0}`, `--rollout-knock {255, 2, 0}`,
`--opp-model {eager, meld}`, `--opp-strength {100, 200, 400}`,
`--max-candidates {4, 6, 8}`; secondarily a `greedy` lane over
`(knock_threshold, score_awareness)`.  Hypothesis: camper
knock model plus stronger sampled hands recovers a large slice of the
gap by itself.  Compute-bound (a day or two of wall clock), zero risk,
and it localizes how much gap remains for real designs.

*Gate met, hypothesis refuted.*  At 2000 games per arm on seed 7, the
causal core ranked by the bot's *own* rollout continuation, not by the
modeled opponent's: `rollout_knock_self: 0` won 46.0% (Wilson
43.8–48.1%), `2` won 39.0%, and today's `255` won 34.4%.  Modeling the
opponent as a camper *hurt* — `rollout_knock_opponent: 0` cost about
three points at every self value (31.9/31.2/30.8%).  Refining around
the winner, `opponent_strength_percent: 200` added ~2 points (48.0%,
45.8–50.2%) with 400 no better (47.5%), `max_candidates` measured flat
(6: 45.1%, 8: 46.4%), and `OpponentModel::MeldOnly` lost 2.8 points
(43.2%), reconfirming §6 now that it is paired with a patient self
policy.  Seed 8 confirmed the best arm at 47.7% (45.5–49.9%).  The
secondary `greedy` lane peaked at 29.6% (knock 4, awareness 32) against
the panel's 29.2% default, so no fixed policy closes the gap and the
whole remedy lives in the search.

So §3's diagnosis was half right: the sampled opponent's *hand
strength* is the defect, and the camper knock rule is not — a rollout
opponent who never knocks makes every line look safe and trades the
undercut error for a passivity error.  That raises M2's importance and
lowers M3's: calibrated sampling is now the load-bearing change, while
archetype inference over *knock policy* has no measured value yet.  The
`opp_strength` probe bounds what crude hand inflation buys at roughly
two points, which is M2's headroom to beat.

The same arm also beats today's default *everywhere else*, measured
identically at 2000 games on seed 7: 71.3% vs `EaaiSimpleBot` against
the default arm's 59.0%, and 76.5% vs `gold-paper` against 67.2%.  This
is a general strength gain, not an anti-MARJJ exploit, and it is
therefore a candidate default change — which by house rule needs the
full panels (`bench-strong.sh`, `bench-panel.sh`) and a `cargo bench`
latency check, since `rollout_knock_self: 0` lengthens every rollout.
Promoting it is M4 work arriving early; nothing ships on `tune`
evidence alone.

**M2 — calibrated sampling (design §D3).**  Replace best-of-k-uniform
with sampling toward baked deadwood-by-turn curves measured from
archetype self-play, so a sampled world's opponent is as developed as a
real one at that point in the round.  This is what prices undercut risk
correctly, and the same instrumentation yields the knock-hazard curves
M3 needs.

*Gate missed against the target; the change is real but points the other
way.*  Two findings, in order.  First, the gap is twice what §3 assumed —
at a twelve-card pile best-of-k models the opponent at 28 deadwood where
a hand played since the deal carries 13 — and §D3's *selection* toward a
target cannot close any of it, because the minimum of k draws already is
the closest draw to any target below it.  Sampling has to construct a
developed hand, which `MonteCarloBot::develop` does; sampled worlds now
land at 13.0 ± 2.  Second, having done so: against `marjj-v5-surrogate`
calibration measured **−1.2 points** of game win share (48.2% against
47.0% over 8000 games a side, pooled over seeds 7, 8 and 9; the
per-seed deltas were +0.2, −1.5 and −1.9, the largest sample the most
negative), while gaining **+3.2** against `EaaiSimpleBot` (69.1% to
72.3%) and **+2.0** against `gold-paper` (75.2% to 77.2%), both seeds
agreeing in direction.  Latency cost is 2.5% at 128 samples.

So a realistic opponent hand is worth two to three points against
opponents who knock, and costs about one against the camper this plan
targets — the exact opposite of §3's undercut diagnosis, which predicted
calibration would pay off *most* against MARJJ.  `hand_calibration`
therefore ships default off: the goal is parity with MARJJ without
giving back strength elsewhere, and this trades the target for the
guards.  It is a documented option for general play, not a step toward
parity.

That leaves the parity gap unexplained by this plan's model of it.  Both
halves of §3's diagnosis have now been measured and both are wrong: the
camper knock model hurt (M1) and the calibrated hand hurts (M2), yet
patience alone reached 48%.  Before M3 builds inference over exactly
those two axes, the next step should be a diagnostic pass over the
rounds we lose — what actually decides them — rather than more
machinery aimed at a diagnosis that has twice failed to hold.

**M2.5 — finish-channel diagnostic.**  A dedicated benchmark observer
replayed the default and `hand_calibration: true` `mc:128` arms against
MARJJ for 2000 mirrored game pairs per arm on each of seeds 7 and 8.  It
used the exact EAAI rules and scored-round dealer alternation, common
random numbers across arms, and inspected hidden hands only outside the
strategies around normal `Table::step` calls.  The run is diagnostic, not
a replacement for the fixed panel.

The default arm won 47.7% (46.6–48.7%) of 8000 games; calibration won
47.0% (46.0–48.0%).  The paired delta was -0.69 points of game win share
(95% pair-cluster interval -2.02 to +0.64; exact arm-sign p = .161), with
seed deltas +0.4 and -1.8.  This alone reconfirms the negative pooled
direction without promoting it to a new strength claim.  Raw score margin
moved from -6.21 to -7.82 points/game, a -1.61 delta (-3.44 to +0.21).

The point accounting supplies the diagnosis.  Calibration made 7146
non-gin knock attempts where default made 17 770 and cut their undercut
rate from 29.2% to 19.3%.  Avoided undercuts recovered 13.71 points/game,
but forfeited knock wins cost 12.23.  It gained 10.90 points/game through
2842 additional own gins while losing 14.47 through 3897 additional MARJJ
gins.  Those MARJJ gin losses were the largest channel on both seeds
(-13.88 and -15.07 points/game), satisfying M2.5's predeclared stability
gate.  The same shift raised rounds with a declined legal knock from
21 849 to 31 034 and dead hands from 1800 to 3982.

So calibration does price undercut risk: it sharply reduces both attempts
and failures.  It loses because it over-corrects into patience, replacing
profitable knocks with longer gin races whose extra losses outweigh both
the saved undercuts and extra own gins.  The M3 design still has no
profitable camper leaf to infer toward, so M3 remains blocked pending a
separate redesign that preserves undercut selectivity without wholesale
passivity.

**M3 — adaptive archetype inference (design §D4, §D5).**  The bot
tracks, across the rounds of a game, when the opponent knocks and how
they draw; classifies them over a small archetype set (eager / balanced
/ camper); and samples worlds from the posterior mixture with each
archetype's rollout policy and hand calibration.  Ships behind a new
`McConfig` knob, default off, so the measurement compares like with
like.  A fresh game starts at today's behavior and converges within a
round or two of evidence — campers reveal themselves fast.  *Do not
implement this as written:* M2.5 found the calibrated camper branch
trades undercuts for a still larger gin-race loss, so classification has
no stronger MARJJ arm to select.

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
  monotone in strength.  M2 is the second instance of the same lesson
  and the stronger one: sampling *correctly developed* opponent hands
  gains points against knockers and loses them against the camper.  A
  more truthful belief is not automatically a better one; only the
  measurement says.
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
M2.5 rejected the original single-cause diagnosis: accurate hand strength
fixes the undercut channel but overprices patience against a gin camper.
The next design must first produce a fixed arm that retains that undercut
selectivity without losing the gin race; inference, per-card posterior
sampling, and offline archetype policies stay out of scope until such an
arm exists.  Learned evaluation remains the last resort.
