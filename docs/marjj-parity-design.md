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

- Mount the benchmark tree narrowly — `#[allow(dead_code)] #[path =
  "support/strong/mod.rs"] mod strong;` — rather than `mod support;` as
  `arena.rs` does, which would also pull in an unused `arena_stats`.
  The precedents are `tests/strong_games.rs` and the two report
  examples.
- Add one `Opponent::Strong(&'static str)` variant carrying the arena's
  own spec, parsed from `--opponent gold-paper` and `--opponent
  marjj-v5-surrogate`, constructed through `make_bot` with the per-game
  seed the existing `OPPONENT_STREAM` offset already provides, so every
  arm faces the same opponent on the same deals.  Spelling the specs as
  the arena spells them means the same token is pasted into a sweep and
  into the panel that confirms it.  `gold-paper` costs one extra match
  arm and completes plan §5's guard matrix, which `tune` otherwise
  cannot screen without an hours-long arena run.

**The dealer protocol is part of M0, not a detail.**  `tune` plays games
through the library's `play_game`, which always builds `Table::new(..)`
and therefore always uses `DealerRotation::WinnerDeals`; there is no
library API for the challenge rotation, which is why `arena.rs` carries
its own `alternate_after_scored_round_game`.  Since `src/value.rs` keys
the Monte Carlo game-value table by `(Rules, DealerRotation)` and bakes
a separate DP solution for each, a sweep under `--rules eaai` alone
selects arms against a *different value function* than the published
panel measures.  So `tune` also gains `--alternate-dealer` and a ~15-line
game loop of its own — it needs only the winner, so none of the arena's
`PlayedGame`/`Outcome` bookkeeping comes along — with the unflagged path
still calling `play_game`, which keeps it byte-identical by construction.

Not adopted: the arena's mirrored pairs.  Arms are already paired to each
other by common random numbers, which is what ranking needs, and
mirroring without pair-cluster inference would make `tune`'s Wilson
interval — which assumes independent games — report a width it has not
earned.  `tune` ranks; `arena` publishes.

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
| `rollout_knock_opponent` | 255, 0 | 0 *is* the gin-camper model: rollout opponents knock only at deadwood 0; 255 is today's knock-ASAP.  Any threshold at or above 10 is the same policy — knocking is legal only at deadwood ≤ 10 — so the grid's original 10 was a duplicate arm and measured byte-identically to 255. |
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

### The measurement that changed this design

The problem above is real and *twice as large as stated*: at a
twelve-card pile the best of twelve uniform draws carries 28 deadwood
where a hand played since the deal carries 13
(`calibrated_worlds_price_a_developed_opponent` measures both).

But selecting toward a target — this section's original design — cannot
fix it.  The minimum of k uniform draws is already the closest draw to
any target below it, so "keep the closest" is *identical to best-of-k*
whenever the target sits under the best draw, which is every position
that matters; early in the round, where the target sits above, it merely
picks a weaker hand.  Nor can more draws close the gap: the minimum of k
falls logarithmically in k, which is exactly why
`opponent_strength_percent` bought two points at 200 and nothing at 400.

Sampling therefore has to **construct** a developed hand, not select one.

### Design (as built)

1. **One baked curve.**  `CALIBRATED_TARGET` in `src/mc.rs`: mean
   deadwood of the waiting seat's ten cards by discard-pile length, from
   5000 seeded camper-versus-camper rounds under `eaai_rules()`, both
   seats at `knock_threshold: 0`.  `Sim::rollout` gains an observed
   variant so the sampler can watch a round develop; `rollout` is that
   function with an inert probe.  The `src/value.rs` precedent holds:
   `curve_matches_fresh_sampling` guards the checked-in numbers and an
   `#[ignore]`d `regenerate_curve` reprints them.
2. **Development.**  `MonteCarloBot::develop` takes the best-of-k draw
   and swaps cards against the unseen pool, accepting a swap when it
   moves the hand's deadwood *closer to* the target, under a 64-attempt
   cap.  Acceptance is by distance rather than by deadwood because one
   swap can complete a meld and overshoot — an overestimate of the
   opponent is no better than the underestimate it replaces.
3. **Pile length, not turn index.**  The curve is measured against the
   same quantity the `View` exposes, so a pile that shrinks when the
   opponent takes reads off the deadwood that hands with that pile
   length really carry.  No turn counter, so M2 does not need `D5`.
4. **Knob surface.**  `McConfig::hand_calibration: bool`, default
   `false`, pinned by the default-pinning test.
   `opponent_strength_percent` keeps its meaning on both paths: it is
   how many hands a world draws before development starts.

One archetype, not three: M1 refuted the archetype *knock policy*, and
until inference (D4) has measured value there is nothing to select
between.  `Option<Archetype>` is the upgrade if D4 ever earns it.

Curves for MARJJ specifically are *not* baked — the camper archetype
is measured from the crate's own policy family, in-tree and
regenerable, which both avoids overfitting the surrogate's literal
constants and keeps everything reproducible from `src/` alone.  The
arena can grow a throwaway diagnostic dump (it owns the `Round`, so it
can log the true hidden deadwood by turn) to *validate* the camper
curve against the surrogate's reality; the dump is scaffolding, not a
shipped interface.

Cost: up to 64 extra `deadwood` calls per world against today's twelve,
so this is a real sampling cost rather than the free one first
estimated — but rollouts dominate a decision, and `cargo bench` puts it
at 2.5% for a 128-sample turn.  Latency is not what keeps this knob off.

### Measured outcome

The plan's M2 entry carries the numbers.  In short: it works as
designed — sampled worlds land at the measured deadwood instead of 15
points above it — and being right about the opponent's hand is worth
+3.2 points against `EaaiSimpleBot` and +2.0 against `gold-paper` while
costing 1.2 against `marjj-v5-surrogate`.  The knob ships default off
because this plan's goal is the camper, and D4's premise — that
per-archetype hand calibration is what a posterior should be selecting
between — is now measured as *negative* for the one archetype that
matters.

### M2.5 diagnosis

The fixed two-arm diagnostic explains the negative result rather than
reversing it.  Calibration cut non-gin knock attempts from 17 770 to 7146
and their undercut rate from 29.2% to 19.3%, recovering 13.71 points/game
from avoided undercuts.  But fewer knock wins cost 12.23 points/game, and
the longer gin races gained 10.90 through extra own gins while losing 14.47
through extra MARJJ gins.  The MARJJ gin channel was the largest change in
the same direction on both seeds.  Calibration is therefore not failing to
model undercut risk; it is over-correcting into passivity.

That leaves D4 without a profitable camper arm.  Do not implement the
posterior mixture below as written until a separate measured design keeps
calibration's undercut selectivity without its gin-race loss.  Classification
cannot add strength when the branch it would select is weaker.

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
  cheaply.  M6 measured 256 at 51.2% (50.1–52.2%) against MARJJ, +4.3
  points over the matched 128-sample anchor, while clearing the EAAI
  and Gold guards.  Its hard-decision latency was 1.99× and whole-game
  throughput was 3.0× slower, so 256 identifies search budget as the
  gap but does not cheaply clear the default-latency constraint.  Skip
  512 unless analysis-only strength above parity becomes a goal.
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
