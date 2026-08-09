# MARJJ parity: technical designs

Companion to the [parity plan](marjj-parity-plan.md), which owns the
goal, the phase gates, and the measurement discipline.  This document
specifies the changes themselves: what each one touches, the API it
adds, and what it must prove before merging.  Section numbers (§D1…)
are the ones the plan's phases cite.

## D1. Point the sweep harness at the surrogate (phase M0)

`examples/tune.rs` sweeps `McConfig` and `HeuristicConfig` arms against
one fixed opponent, but its `Opponent` enum stops at `Greedy`, `Mc`,
and `Eaai`.  The arena already constructs the surrogate
(`support::strong::make_bot("marjj-v5-surrogate", seed)`); `tune` gets
the same access:

- Add `mod support;` to `tune.rs` exactly as `arena.rs` does
  (`examples/support/mod.rs` is shared by construction).
- Add `Opponent::Marjj`, parsed from `--opponent marjj`, constructed
  through `make_bot` with the per-game seed the existing
  `OPPONENT_STREAM` offset already provides, so every arm faces the
  same opponent on the same deals.

Non-changes, deliberately: the surrogate itself stays frozen — it is
the measured yardstick, and edits would orphan the published panel —
and stays outside the library API and the interactive player list.  No
arena changes; panel-grade claims use the arena's existing
`marjj-v5-surrogate` spec.  If M3's shipping shape ever needs a
non-default `McConfig` in a panel, add one named arena spec then, not
speculatively.

## D2. The knob matrix (phase M1, no library code)

Every lever below already exists; M1 is measurement, not engineering.
The sweep grid, with rationale:

| Knob | Values | Why |
|------|--------|-----|
| `rollout_knock_opponent` | 255, 10, 0 | 0 *is* the gin-camper model: rollout opponents knock only at deadwood 0.  10 models an any-legal-knock opponent; 255 is today's knock-ASAP. |
| `rollout_knock_self` | 255, 2, 0 | Patient own continuations become correctly valued once the modeled opponent stops knocking first in every rollout.  The old finding that 2 is a baseline exploit was measured under `opp=255`; the interaction is the point. |
| `opponent_model` | eager, meld | MARJJ's draw gate is meld-only.  Alone this measured flat against the baseline; paired with a camper knock model it may matter. |
| `opponent_strength_percent` | 100, 200, 400 | Scales the best-of-k hand bias.  400% late in a round is ~30 draws — a crude stand-in for D3, and a cheap upper-bound probe on what calibration can buy. |
| `max_candidates` | 4, 6, 8 | Anti-camper lines (safety sheds, two-way-draw keepers) are not always among the four lowest-deadwood sheds. |
| `gate_z` | 2.0 only | Held fixed.  Loosening measured weaker; the fix is world realism. |

Not the full cartesian product: sweep `opp-knock × self-knock` first
(the causal core, nine arms), then refine around the winner one knob
at a time.  A secondary `greedy` lane sweeps `(knock_threshold,
score_awareness)` against MARJJ — cheap, and it calibrates how much of
the gap is search versus pure policy.  Interesting wrinkle the sweep
should confirm or kill: against a camper, *early* knocks (before the
opponent's hand develops) may be fine while *late* knocks are the
poison — if so, fixed thresholds cannot express the right policy and
the case for M2+M3 is airtight.

## D3. Calibrated opponent-hand sampling (phase M2)

### Problem

`sample_worlds` keeps the lowest-deadwood of `pile_len / 2` uniform
draws.  That schedule was tuned to beat "uniform is far too weak", but
it still badly underestimates a developed hand late in a round: a
camper on turn 10 holds near-gin, while the best of ~7 uniform
ten-card draws from ~20 unseen cards sits in the twenties.  Undercut
risk is therefore priced near zero exactly where it is highest — the
32% undercut rate against MARJJ is this bug wearing a costume.

### Design

Sample toward a **target deadwood** instead of the minimum:

1. **Baked curves.**  For each archetype (eager / balanced / camper —
   concretely, the rollout policy at `knock_threshold` 255 / 4 / 0,
   with the matching draw rule), measure hidden-hand deadwood *mean
   and spread by turn index*, plus the **knock-hazard curve** —
   P(first knock at turn t) — from seeded self-play driven by the
   existing `Sim`/`Table` machinery.  Bake the small tables into the
   crate with an `#[ignore]`d `regenerate_*` test that reprints them,
   exactly the `src/value.rs` precedent (`regenerate_baked`).
2. **Sampling.**  Per world: draw a target from the archetype's curve
   at the current turn (clamped), draw the same k candidate hands as
   today, keep the one whose `deadwood(known | hidden)` is *closest to
   the target* rather than lowest.  Diversity across worlds now comes
   from the target draw, not only from the uniform hands, so worlds
   spread realistically instead of stacking at best-of-k.
3. **Turn index.**  The bot's own `play_turn` count within the round
   (reset per D5); the opponent's is that ±1.  `discard_pile().len()`
   is not monotone (takes pop it), so it stays a fallback only.
4. **Knob surface.**  One added `McConfig` field, e.g.
   `hand_calibration: Option<Archetype>`, `None` meaning today's
   best-of-k (default, pinned by the existing default-pinning test).
   `opponent_strength_percent` keeps its meaning for the `None` path.

Curves for MARJJ specifically are *not* baked — the camper archetype
is measured from the crate's own policy family, in-tree and
regenerable, which both avoids overfitting the surrogate's literal
constants and keeps everything reproducible from `src/` alone.  The
arena can grow a throwaway diagnostic dump (it owns the `Round`, so it
can log the true hidden deadwood by turn) to *validate* the camper
curve against the surrogate's reality; the dump is scaffolding, not a
shipped interface.

Cost: one extra `deadwood` call per candidate hand — the same k calls
as today, plus a target draw.  `cargo bench` before/after.

## D4. Adaptive opponent-archetype inference (phase M3)

### Signals — all information-hygienic, all already visible

Within a round, from the bot's own `View` history: the opponent's
upcard takes and passes (take rate, and whether takes track the
meld-only gate), and *knock silence* — how many opponent turns have
passed without a knock.  Across rounds: when the opponent knocked (the
defender sees `choose_layoff` with the spread; the attacker knows the
opponent never knocked), and how long rounds ran.  Nothing new leaks:
no `View` accessor is added; the bot only remembers what it was
already shown.

### Classifier

Posterior over the same archetype set D3 bakes, updated from the
knock-hazard curves: each opponent turn without a knock multiplies in
P(silence | archetype, turn); an observed knock at turn t multiplies
in its hazard.  Draw behavior nudges eager-vs-meld-only.  Start every
game at a prior concentrated on today's default model, so round one
plays like the current bot; a camper separates from the prior within
one round of silence (their hazard curves diverge fast after turn 3 —
MARJJ's voluntary knocks *only* happen on turns 1–3).  Apply
hysteresis (a minimum evidence count before leaving the prior) so one
odd round cannot flip the model.  The whole thing is a pure function
of the observation history — deterministic under a seeded RNG, per
invariant 6.

The first implementation should be the boring one: counters and a
likelihood table, no Dirichlet machinery, upgraded only if measurement
demands it.

### Using the posterior

Allocate the world budget across archetypes proportionally to the
posterior (deterministic largest-remainder allocation, not an RNG
draw), each world carrying its archetype's `SeatPolicy` — the field
already exists per seat — and its D3 hand calibration.  Candidates
still share every world (common random numbers hold), batching and
elimination are untouched, and the mean over worlds is exactly the
posterior-weighted equity.  If the camper archetype needs MARJJ's
turns-1–3 early-knock quirk, `SeatPolicy` grows an optional
`knock_until_turn` cap — deferred until diagnostics show those 9% of
rounds actually cost points (flagged YAGNI).

### Config and rollout

New `McConfig` field `adaptive_opponent: bool`, default `false` — the
fixed knobs (`rollout_knock_opponent`, `opponent_model`,
`hand_calibration`) then act as the inference's fallback prior rather
than dead settings.  Adding a field to a `#[non_exhaustive]` struct is
semver-minor.  M3 measures the knob on; M4 flips the default in its
own step, because a `McConfig` default change is a strength change by
house rule and owes the full `measure-strength` procedure.

## D5. `Strategy::begin_round` (with M3)

Stateful strategies currently infer round boundaries from heuristics —
the surrogate sniffs a 31-card stock and a changed hand, and documents
a pathological case where stale history survives.  The clean fix is a
driver-announced boundary:

```rust
/// Called by the driver before the first decision of each round.
fn begin_round(&mut self, _view: &View<'_>) {}
```

A defaulted trait method is semver-minor and object-safe; the driver
(`play_round`, and `play_game` per round) calls it for both seats.
The adaptive bot resets its per-round state there and carries its
cross-round posterior forward.  Third-party stateful bots get the same
benefit.  The surrogate is *not* retrofitted — frozen yardstick — and
`tests/driver.rs` grows coverage that the hook fires once per round
per seat, before any decision callback.

## D6. Tactical extras (post-parity, or if a few points short)

- **Layoff-minimizing knock spread.**  MARJJ picks, among its minimal
  partitions, the spread that minimizes the opponent's expected
  layoff; our knocks always use `best_melds`' single partition.
  Needs minimal-partition enumeration in the engine —
  `all_minimum_melds` currently lives in `examples/support/strong/`
  precisely because the library lacks it, so this either promotes an
  equivalent into this crate or (better) upstreams enumeration into
  gin-rummy under the sibling-crate protocol.  Real-bot-only; rollout
  knocks keep `best_melds` (own-future fidelity is not the binding
  constraint).  Expected small; measure before keeping.
- **Sample-budget curve.**  Chart 128 → 256 → 512 against MARJJ once
  the model is right — batching absorbs much of the cost on easy
  decisions, and the `parallel` feature covers analysis use.  The
  interactive default stays 128 unless the curve says otherwise
  cheaply.
- **Undercut-margin knock guard in `HeuristicBot`.**  Only if the M1
  greedy lane shows fixed policies can profit; otherwise skip — the
  heuristic's job is to stay simple.

## Invariants impact (CLAUDE.md §Invariants)

1. **Information hygiene** — no new `View` accessor anywhere in
   D1–D6; the adaptive state is a fold over views the seat legally
   received.  `tests/view.rs` unchanged plus a new assertion that
   `begin_round` receives the seat's own view.
2. **Unseen identity** — untouched; sampling still partitions
   `unseen` between hidden hand and stock (the existing
   `sampled_worlds_are_consistent_with_the_view` test keeps guarding
   D3's sampler).
3. **Driver bookkeeping** — no new actions; D5 adds a callback, not a
   ledger entry.  Driver tests extend for the hook's timing.
4. **`Sim` mirrors `Round`** — no mechanics change in any phase, so
   the equivalence proptest stands unweakened and the baked value
   models stay valid.  D3's curves are *priors*, not mechanics
   mirrors: their drift guard is the strength panel, and their
   `regenerate_*` test reprints them on demand.
5. **Greedy core doubles as rollout policy** — `best_shed`,
   `improves`, `joins_a_meld`, `greedy_layoff` are untouched.
   `SeatPolicy` gains at most the optional `knock_until_turn` cap,
   default-inert, pinned alongside the existing default-pinning
   test; the equivalence proptest pins the default policy exactly as
   before.
6. **Determinism** — the classifier is a pure fold; sampling draws
   only from the owned RNG; world allocation across archetypes is
   deterministic.  Seeded-replay tests extend to the adaptive knob.

## Test plan

- Default-pinning: every new `McConfig` field's default reproduces
  current behavior bit for bit (extend
  `config_default_pins_the_historical_constants` and the seeded
  decision-diff test).
- D3: property test that a calibrated world is still a legal
  partition containing `opponent_known`; a fixture showing late-round
  calibrated hands carry materially lower deadwood than best-of-k.
- D4: a scripted camper opponent flips the posterior within two
  rounds; a scripted eager knocker does not; seeded replay of an
  adaptive game is bit-identical.
- D5: hook fires once per round per seat before any decision, in
  `play_round` and in every round of `play_game`.
- The strength tripwire, both fixed panels, and `cargo bench` per the
  plan's gates — numbers live in the plan, not here.
