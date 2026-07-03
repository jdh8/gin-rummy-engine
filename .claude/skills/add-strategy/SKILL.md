---
name: add-strategy
description: Add a new bot (Strategy implementation) to gin-rummy-engine, or extend an existing bot with new tuning knobs. Use when implementing a new playing strategy, AI opponent, or bot variant.
---

# Add a new Strategy

## The contract

A strategy is four decisions against a `View` (`src/strategy.rs`):

- `offer_upcard`: take or pass the initial upcard.
- `choose_draw`: stock or pile.  Not consulted on the forced stock draw
  after both players pass the upcard, so `TakeDiscard` may be treated as
  always available.
- `play_turn`: the hand holds 11 cards; `view.taken_discard()` may not be
  shed this turn.  Return `Discard`, `Knock` (the chosen melds fix what
  the defender may lay off), or `BigGin`.
- `choose_layoff`: called repeatedly with a refreshed view until it
  returns `None`; meld indices follow `View::spread` enumeration order
  and are stable across layoffs.
- `name`: the display name used by arena output.

Hard rules:

- Decide from the `View` alone.  Never accept a `Round`, `Table`, or the
  opponent's hand; if the `View` lacks information you want, it is almost
  certainly information the seat may not legally see.
- The driver rejects illegal choices as `EngineError::IllegalAction`
  without corrupting the table, but bots shipped in this crate must never
  trigger it — the tests treat a rejection as a bug.
- A randomized bot owns its RNG as a constructor argument (like
  `MonteCarloBot<R: Rng>`) so that seeded runs replay identically, and is
  gated behind `#[cfg(feature = "rand")]`.

## Checklist — every place a new bot touches

1. `src/<name>.rs`: the implementation plus unit tests.  Reuse the greedy
   primitives `best_shed`, `improves`, `greedy_layoff` from
   `src/heuristic.rs` before writing new ones.
2. `src/lib.rs`: `mod` and `pub use`, with `#[cfg(feature = "rand")]` if
   gated.
3. Doc comments on every public item — a missing one fails CI.  Config
   structs are `#[non_exhaustive]` with builder-style consuming setters.
4. `make_bot` in **both** `examples/play.rs` and `examples/arena.rs`,
   including their error-message bot lists.
5. The "Bots" section of README.md (the README doubles as the crate's
   front-page rustdoc).
6. `benches/decision.rs` if per-decision speed matters.
7. CHANGELOG.md under `[Unreleased]`.
8. Measure it with the `measure-strength` skill.  Any strength claim in
   docs ("wins X% against Y") must come from arena runs with the command
   and sample size noted in the commit message — never from a few games.
