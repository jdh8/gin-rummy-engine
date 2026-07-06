# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A solver/hint view surfaces the Monte Carlo bot's read on a decision.  The
  new `MonteCarloBot::assess` method and public `Assessment` type return every
  candidate move for the current phase with its equity (its chance to win the
  game) and expected round points, ranked, with the bot's own pick flagged —
  the numbers the bot already computes to choose, now available to a caller.
  The terminal `play` example shows the table on a `hint` command (or `h`,
  except on the discard prompt where a lone `h` names a heart), and the
  browser front end on a `Hint` button (or the `h` key), so a human can weigh
  each move without the bot playing it for them.

### Changed

- `MonteCarloBot`'s assumed opponent hand now keeps improving for the
  whole round instead of leveling off about a third of the way in.  Its
  rollouts model the hidden hand as the best of several drawn hands, more
  of them the deeper the pile has grown; that scaling no longer stops
  increasing partway through, so late-round equity and EV reads no longer
  assume an opponent who stopped getting better early.

- The browser front end now hides the move log by default, so a long game
  no longer grows a wall of text down the right edge; the table takes the
  full width instead.  A `Show log`/`Hide log` button in the header (or the
  `l` key) reveals it on demand.  The preference is not remembered — every
  load starts hidden, matching the fresh game each reload deals.

## [0.1.2] - 2026-07-05

### Changed

- `MonteCarloBot` now plays for the game, not just the round.  Each
  rollout result lands on the running game totals: a result that reaches
  `game_target` counts as the game win or loss it is, and anything short
  of a clinch counts its round points as before.  Immediate boxes
  (palace-style rules) are priced in; deferred boxes and game bonuses
  never decide who goes out first and are ignored.  Mid-game decisions
  are unchanged by construction, and so is play outside a game; the bot
  deviates exactly when a round can end the game — it takes a knock that
  clinches instead of milking a bigger score, and it defends the round
  when losing it would hand the opponent the game.  Aggregate strength
  against the default heuristic is unchanged within measurement error:
  mc:64 still wins ≈54% of modern-rules games and ≈61% of palace games,
  and mc:128 ≈65% of decisive standalone rounds.
- `HeuristicBot` now plays for the game, not just the round.  Its default
  configuration is retuned by whole-game self-play: it holds past the first
  legal knock (`knock_threshold` 4, was 10) rather than banking a small
  knock every hand, and it reads the running game score (`score_awareness`
  40, was 0 — a new knob), knocking sooner when ahead to lock in a lead and
  holding out for gin when behind.  The score shift is keyed to the
  leader's distance to the winning line, not the raw margin, so the same
  lead bends the knock threshold ever harder as the game nears its end — a
  nudge early on becomes a knock at any deadwood once the front-runner is a
  hand from `game_target`.  Over full games this lifts its win rate from
  roughly 42% to 50% against `MonteCarloBot` and to about 60% against the
  previous default.  A round played outside a game (no scoreboard) is
  unaffected; `HeuristicBot::new()` and `HeuristicConfig::default()` change
  accordingly.
- Require gin-rummy >= 0.1.2, whose `best_melds` now breaks equal-deadwood
  ties in favor of runs over sets.  Deadwood totals are unaffected, but a
  layoff or a knock's reported melds may pick a different (equally optimal)
  arrangement on hands where a run and a set tie.

### Fixed

- The `play` example and the browser front end now attach the running game
  totals to each round's table, so score-aware strategies — the retuned
  `HeuristicBot` and now `MonteCarloBot` — actually see the score when
  playing a human.  Previously they played every round as if the game were
  level.

### Added

- `View::game_scores()` reports both running game totals, this seat's
  first — the whole scoreboard is public, so information hygiene holds —
  giving strategies the distance to `game_target` that `game_margin`
  alone cannot recover.
- `View::game_margin()` reports the seat's running lead in the game score —
  positive ahead, negative behind, zero for a round played on its own — so a
  strategy can bank a lead or gamble from behind.  The game score is public
  to both players, so this keeps information hygiene intact.
- `Table::scores()` attaches the running game totals to a round; `play_game`
  now supplies them, so any bot it drives sees the live margin.
- `HeuristicConfig::score_awareness`, the knob that couples the knock
  threshold to the game score, scaled by the leader's distance to
  `game_target` so it bites hardest as a game nears its end.  Zero
  reproduces the previous score-blind play.
- A `tune` example: whole-game A/B self-play that sweeps the heuristic's
  knock knobs against a fixed opponent (`greedy` or `mc`), reporting each
  arm's game-win rate with a Wilson interval.  Each arm's games are seeded
  by index and played in parallel across the CPUs, so the counts stay
  deterministic; it picked the new defaults.
- The browser front end has a Difficulty dropdown (Easy/Medium/Hard) in the
  header, so picking an opponent no longer requires editing `app.js`.  The
  three tiers are distinct opponents rather than Monte Carlo sample-count
  variants: a `newbie` heuristic that knocks at the first legal chance and is
  blind to both the game score and discard safety, the score-aware default
  heuristic, and `mc:128`.  The `play` example accepts `--bot newbie` as well.

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

[Unreleased]: https://github.com/jdh8/gin-rummy-engine/compare/0.1.2...HEAD
[0.1.2]: https://github.com/jdh8/gin-rummy-engine/compare/0.1.1...0.1.2
[0.1.1]: https://github.com/jdh8/gin-rummy-engine/compare/0.1.0...0.1.1
[0.1.0]: https://github.com/jdh8/gin-rummy-engine/releases/tag/0.1.0
