# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- `HeuristicBot` now plays for the game, not just the round.  Its default
  configuration is retuned by whole-game self-play: it holds past the first
  legal knock (`knock_threshold` 4, was 10) rather than banking a small
  knock every hand, and it reads the running game score (`score_awareness`
  32, was 0 — a new knob), knocking sooner when ahead to lock in a lead and
  holding out for gin when behind.  Over full games this lifts its win rate
  from roughly 42% to 50% against `MonteCarloBot` and to about 60% against
  the previous default.  A round played outside a game (no scoreboard) is
  unaffected; `HeuristicBot::new()` and `HeuristicConfig::default()` change
  accordingly.
- Require gin-rummy >= 0.1.2, whose `best_melds` now breaks equal-deadwood
  ties in favor of runs over sets.  Deadwood totals are unaffected, but a
  layoff or a knock's reported melds may pick a different (equally optimal)
  arrangement on hands where a run and a set tie.

### Added

- `View::game_margin()` reports the seat's running lead in the game score —
  positive ahead, negative behind, zero for a round played on its own — so a
  strategy can bank a lead or gamble from behind.  The game score is public
  to both players, so this keeps information hygiene intact.
- `Table::scores()` attaches the running game totals to a round; `play_game`
  now supplies them, so any bot it drives sees the live margin.
- `HeuristicConfig::score_awareness`, the knob that couples the knock
  threshold to the game score.  Zero reproduces the previous score-blind
  play.
- A `tune` example: whole-game A/B self-play that sweeps the heuristic's
  knock knobs against a fixed opponent (`greedy` or `mc`), reporting each
  arm's game-win rate with a Wilson interval.  It picked the new defaults.
- The browser front end has a Difficulty dropdown (Easy/Medium/Hard, mapping
  to `greedy`/`mc:16`/`mc:64`) in the header, so picking an opponent no longer
  requires editing `app.js`.

## [0.1.1] - 2026-07-05

### Changed

- Require gin-rummy >= 0.1.1, whose `Card`, `Meld`, and `Melds` now display
  rank-first (`T♥`, `5♠6♠7♠`).  Cards surfaced through the `View` API print in
  this order; parsing still accepts either order.
- The `play` example and the browser front end now spell out a scored round as
  `earned + bonus = total` (`You gin (24 + 25 = 49)`, `You undercut (8 + 25 =
  33)`), so the printed number matches the score change instead of showing only
  the opponent's deadwood and silently omitting the gin or undercut bonus.
- The `play` example prints cards rank-first (`T♥`, matching gin-rummy's new
  Display) and shows your hand on one line: the melds, then just the loose
  deadwood ordered by rank by default, with a `sort` command to switch the
  deadwood between by-rank and by-suit while you play.
- The `play` example takes moves more tersely: type a card to discard it (no
  `discard` command), and name a card by a lone rank or suit (`5`, `♠`, `t`)
  when your hand holds exactly one match.  `knock` (or `n`, its highlighted
  hotkey) always auto-sheds the smallest knockable deadwood — the shed goes
  face down and never reaches the opponent, so it is never a real choice.
- The `play` example quits only on the full word `quit`, not a bare `q`, which
  now names your only queen the way `k` names your only king; end-of-input
  (Ctrl-D) closes the prompt line before exiting.

### Added

- The `play` example highlights the card you just drew in your hand, so it is
  easy to track from turn to turn even after it slots into a meld.

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

[Unreleased]: https://github.com/jdh8/gin-rummy-engine/compare/0.1.1...HEAD
[0.1.1]: https://github.com/jdh8/gin-rummy-engine/compare/0.1.0...0.1.1
[0.1.0]: https://github.com/jdh8/gin-rummy-engine/releases/tag/0.1.0
