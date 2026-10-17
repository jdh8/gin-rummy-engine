# Crate improvement report

Reviewed on 2026-10-18 at commit
`729cdabaa3e72b2b52ea3a1c08907500d5095966` (version 0.3.1).

The review proposed five priorities: configuration validation, a missing
CI feature check, broader benchmarks, explicit round boundaries, and a
focused strength experiment.  All five have since been addressed; each
section ends with its status.

## 1 Validate Monte Carlo configuration

Two public configuration edge cases are confirmed:

- Setting `McConfig::max_candidates = 0` causes a panic during ordinary
  discard selection.  `hint_candidates` truncates the discard list to
  zero, and `choose` passes an empty score list to `recommended`, which
  indexes its first entry.
- Calling `MonteCarloBot::samples(0)` makes `assess()` return `NaN` for
  both equity and expected points on an opening decision.  No worlds are
  scored, so `rank` divides both totals by zero.

Two isolated tests reproduced these behaviors with a sorted deck dealt
round-robin and `StdRng` seed 1.  The tests passed by asserting the current
failure modes; they did not verify a fix.

Define valid configuration ranges and enforce them consistently through
`with_config` and builder setters.  Add focused regression coverage for
the chosen rejection or normalization behavior.  Keep valid defaults and
seeded decisions unchanged.

Source: [`src/mc.rs`](../src/mc.rs).

*Status: done in `36e787c`.*  Zero sample and candidate counts are raised
to one, so neither configuration panics nor yields `NaN`.

## 2 Check the lightweight build in CI

The documented verification gauntlet includes
`cargo check --no-default-features`, but
[`rust.yml`](../.github/workflows/rust.yml) currently runs only default
and all-feature tests, plus the other lint and documentation jobs.
Neither test configuration disables `rand`.

Add the missing check to protect the supported heuristic-only build
against accidental dependencies on optional features.

*Status: done in `5e4a42a`.*

## 3 Broaden decision benchmarks

The Monte Carlo cases in
[`benches/decision.rs`](../benches/decision.rs) all use one opening
discard position.  They measure several sample budgets, but do not cover
draw decisions or the changing costs of later positions.

Add a small deterministic collection covering draws, late-round discards,
and knock choices.  Use those cases alongside whole-game throughput when
judging an optimization.  Consult the
[performance backlog](performance-backlog.md) first: it already records
implemented improvements and rejected experiments, including early
acceptance and deadwood-only calibration swaps.  The allocation profile
does not currently justify fixed-size rollout buffers.

*Status: done in `c9c14e5` and `b49edec`.*  The later positions are
reached by replaying recorded moves.

## 4 Give stateful strategies an explicit round boundary

[`Strategy`](../src/strategy.rs) has no round-start callback.  Stateful
bots infer resets from upcard callbacks or stock counts; a dealer can
miss the upcard callback when the opponent takes the initial upcard.

The existing [`begin_round` proposal](marjj-parity-design.md#d5-strategybegin_round-with-m3)
would provide a default trait method receiving only the seat's `View`.
Define its behavior for both whole-game driving and direct `Table::step`
use, including retries, so each seat gets one notification before its
first decision in a round.  Preserve information hygiene and test both
dealer protocols.  Keep the frozen benchmark surrogate unchanged, as the
proposal specifies.

*Status: done in `13ac2e6`.*

## 5 Test a focused strength hypothesis

The retained [calibration diagnosis](marjj-parity-design.md#m25-diagnosis)
found that more realistic opponent hands reduced undercuts but discouraged
knocking enough to lose more through prolonged gin races.  That evidence
does not establish a stronger replacement policy.

The next research step is a small fixed-policy experiment aimed at
retaining undercut selectivity without the gin-race loss.  Measure that
arm before adding adaptive opponent classification.  Follow
[measure-strength](../.claude/skills/measure-strength/SKILL.md), including
whole-game comparisons, explicit seeds, and mirrored-pair inference.
The MARJJ evidence concerns a host-engine surrogate, not an established
championship binary; the
[strong-opponent qualifications](strong-opponents.md) continue to apply.

*Status: tested and refuted.*  The softened-target arm, M2.6 in the
[parity plan](marjj-parity-plan.md), measured +0.48 points (−1.04 to
+2.01, p = .55) against MARJJ on a confirmation seed at `mc:128`, and
−0.87 points (−2.38 to +0.65, p = .18) at the 512-sample default.  Its
knob was discarded.  A further strength attempt needs a mechanism other
than calibration strength.
