# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- The `play` example prints cards rank-first (`T♥`, matching gin-rummy's new
  Display) and lists your hand ordered by rank by default, with a `sort`
  command to switch between by-rank and by-suit while you play.
- The `play` example takes moves more tersely: type a card to discard it (no
  `discard` command), and name a card by a lone rank or suit (`5`, `♠`, `t`)
  when your hand holds exactly one match.  `knock` (or `n`) with no card
  auto-sheds to the smallest knockable deadwood.
- The `play` example quits only on the full word `quit`, not a bare `q`, which
  now names your only queen the way `k` names your only king; end-of-input
  (Ctrl-D) closes the prompt line before exiting.

### Added

- Contributor documentation: an expanded CLAUDE.md (crate map, invariants,
  verification gauntlet, conventions) and step-by-step procedures under
  `.claude/skills/` for syncing the forward model, measuring bot strength,
  adding strategies, and cutting releases.  No changes to the crate's API
  or behavior.

## [0.1.0] - 2026-07-04

### Added

- The `Strategy` trait: one method per decision point (upcard offer, draw
  source, discard/knock/big gin, layoffs), object-safe and stateful.
- `View`, an information-hygienic window on a round: own hand, discard pile,
  stock count, the opponent's revealed cards (taken, shed, passed), and the
  `unseen` set that determinization samples from — never the opponent's hand
  or the stock order.
- The `Table` driver owning the `Round` and per-seat knowledge, with
  `step`/`play`, plus the `play_round` and `play_game` conveniences.
- `HeuristicBot`: a deterministic greedy player with knowledge-aware discard
  safety and meld-preserving layoffs, tunable via `HeuristicConfig`.
- `MonteCarloBot` (feature `rand`): flat determinization — samples hidden
  worlds consistent with the view, rolls them out greedily, and maximizes
  expected round points with common random numbers across candidates.
- Examples: `play` (human vs bot in the terminal) and `arena` (bot-vs-bot
  tournaments with win rates and result tallies).

[Unreleased]: https://github.com/jdh8/gin-rummy-engine/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jdh8/gin-rummy-engine/releases/tag/v0.1.0
