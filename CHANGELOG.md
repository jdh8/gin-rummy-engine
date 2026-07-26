# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `McConfig` and `OpponentModel`: public tuning knobs for
  `MonteCarloBot`, in the mold of `HeuristicConfig`.  The knobs expose
  the search's previously hardcoded levers — the per-seat rollout knock
  thresholds (`rollout_knock_self`, `rollout_knock_opponent`), the
  modeled opponent's draw rule (`OpponentModel::Eager`, the historical
  greedy rule, or `OpponentModel::MeldOnly`, the EAAI baseline's
  take-only-into-a-meld rule, now shared with `EaaiSimpleBot` so model
  and baseline cannot drift), the significance-gate width `gate_z`, the
  discard candidate cap `max_candidates`, and the sampled opponent's
  plausibility bias `opponent_strength_percent`.  Every one of these
  knobs defaults to the constant the search already used, guarded by a
  default-pinning test and verified by a seeded arena decision diff, so
  none of them changes play until it is turned.  (`game_value`, listed
  under Changed, is the one default that did move.)
  `MonteCarloBot::with_config` and `MonteCarloBot::config` round out the
  API; `samples(n)` remains as sugar.  First measurements through the
  knobs — mc:64 against the challenge baseline, 3000 deal-paired games
  per arm and seed — found that modeling the baseline's own conservative
  draw rule (`MeldOnly`) is no better than the default eager model
  (53.9% vs 54.8%), so belief fidelity is not monotone in strength, and
  that a nearly-gin-hunting own continuation (`rollout_knock_self: 2`)
  keeps a small repeatable edge over the knock-ASAP default on both
  seeds tried (55.2%/55.7% vs 54.8%/53.9%) while the middling thresholds
  4 and 8 are worse than either extreme.  That edge does not survive
  strong opposition — the same patient continuation scores 47.9% vs
  48.9% for the default against an unmodified mc:64 — so it is an
  exploit of the baseline's knock-ASAP habit, available as a knob for
  anyone targeting that baseline.  No finding clears the bar to change
  a default.
- The `arena` example measures like an instrument now.  Trials are
  seeded by index and fan out across the CPUs (a run takes minutes, not
  hours, at Monte Carlo sample counts); every trial defaults to a
  *mirrored pair* — both bots play the same deal(s) from both seats,
  cancelling deal luck (`--unpaired` restores independent trials) — and
  each run ends with a significance line (paired z and p-value, or a
  sign test unpaired) so a comparison states its own confidence instead
  of leaving two overlapping intervals to the reader.
  `--alternate-dealer` plays whole games under the EAAI challenge's
  protocol (the deal alternates every hand) rather than gin rummy's
  winner-deals-next, which measurably shifts game rates and is required
  for numbers quoted against the challenge literature.  A run without
  `--seed` picks one and prints it, so any result can be reproduced.
- The `tune` example sweeps `MonteCarloBot` knobs: `--mc-samples N`
  makes the candidate a Monte Carlo bot and `--rollout-knock`,
  `--opp-knock`, `--opp-model`, `--gate`, `--max-candidates`, and
  `--opp-strength` sweep `McConfig` fields as comma lists (arms are the
  cartesian product).  The fixed opponent can now be the challenge
  baseline (`--opponent eaai`) and the rules preset `eaai` is accepted,
  so a knob can be tuned against the yardstick it aims at.
- `scripts/bench-panel.sh` regenerates README's benchmark table.  The
  published numbers were previously a hand-kept transcript of runs
  nobody else could repeat; the script pins the bots, the rules, the
  dealer protocol, the seeds and the counts, prints the table on stdout
  and the full arena log on stderr, and stamps the commit it ran at.
  Anyone can now reproduce the crate's claimed strength — the arena is
  deterministic in its seed, so at a given commit the table comes back
  identical — and a claim that no longer reproduces is visible instead
  of quietly rotting.

### Changed

- `EaaiSimpleBot`'s documentation now states what the bot is and how it
  differs from the challenge framework's Java player.  It is an
  implementation of that player's published policy rather than a
  transliteration of its code, and the four places it departs are now
  listed with their consequences — including two that were previously
  unrecorded: the layoff sweep, where this bot's greedy layoff can only
  do better for the defender than the framework's first-fit pass, so win
  rates published against this baseline are conservative rather than
  inflated; and the round's (draw, discard) loop breaker, which keys on
  an ordered pair where the original keys on an unordered one.  The
  `--rules eaai` preset carries box, game, and shutout bonuses the
  framework has none of, which the docs now explain are settled only
  after a player has reached the game target and therefore never decide
  a game.  Behavior is unchanged and every published number stands; this
  audit against the framework's source is what confirms them
  comparable with the challenge literature.
- `MonteCarloBot` now values a decision by its probability of winning the
  **game**, not by the round points it banks.  Short of a clinch the old
  equity was affine in round points, so the search had no sense of the
  scoreboard: a point of a commanding lead counted for exactly as much as
  a point of a hopeless deficit, and the bot neither banked a lead nor
  pressed a comeback.  It now prices a round outcome through a solved
  win-probability function of both scores and the dealer rotation, so it
  plays the lead it has.  Measured against the previous behavior over
  12 000 deal-paired games per seed under the EAAI challenge protocol,
  the new default wins 51.1% and 51.4% of games on two seeds (+2.1 and
  +2.7 points, p = 0.044 and p = 0.009), winning on banked points as well
  as on games, and it does not regress against other opponents: 54.9% and
  54.7% of games against `EaaiSimpleBot` where the old equity scored
  54.8% and 53.9%, and 53.4% and 52.6% against `HeuristicBot` where it
  scored 52.9%.  `McConfig::game_value` selects the behavior —
  `GameValue::Table` is the new default, `GameValue::Affine` restores the
  old one — and the arena's `mca` bot spec plays the affine arm so the
  comparison stays reproducible.

  The win-probability function is a dynamic program over how greedy
  self-play actually ends rounds under a given ruleset, and those
  measurements ship with the crate rather than being taken on first use:
  sampling them costs about two seconds and solving the program from them
  about eight milliseconds, and no interactive caller should pay the
  former on a first decision.  Only the built-in presets and the EAAI
  challenge variant are covered; any other ruleset — one built by hand,
  or by the web front end's rules editor — keeps the affine value rather
  than stalling to measure itself.

- `arena`'s trial seeding is per-index (SplitMix-style), so the deal
  stream depends only on `--seed` and the trial index.  Runs at the same
  seed reproduce exactly across machines and code revisions — old-vs-new
  comparisons at one seed are themselves paired — but a seeded 0.2.0
  arena run is not reproduced by this version.  `--rounds N`/`--games N`
  now count trials (mirrored pairs by default, so twice as many rounds
  or games are played).
- The README benchmarks against `EaaiSimpleBot` are re-settled with the
  new instrument — mirrored pairs, the challenge's alternate-dealer
  protocol, and 8000–12 000 games per matchup where the old table had
  500–600: `greedy` wins 59.7% of games (was 57.4% ± 4.4), `mc:64` 54.8%
  (was 53.3% ± 4.0, now genuinely below greedy rather than within
  noise), and `mc:128` — previously unmeasured over games — 59.6%,
  closing the gap.  `mc:64` still beats `greedy` head-to-head over whole
  games (53.0% of 12 000), so exploiting the weak baseline and winning
  the head-to-head are established as different skills.  No bot behavior
  changed; only the measurement did.

## [0.2.0] - 2026-07-17

### Added

- Oklahoma gin flows through the engine: gin-rummy 0.1.3's
  `Rules::oklahoma` caps the knock limit at the opening upcard's value,
  and `View::knock_limit()` has always been the resolved per-round limit,
  so every shipped bot knocks legally under the variant with no change to
  its decision logic.  The Monte Carlo forward model resolves the same
  limit, and the `Sim`/`Round` equivalence and hygiene proptests now
  exercise both Oklahoma ace schools alongside the three presets.
- `EaaiSimpleBot` (feature `rand`): a port of `SimpleGinRummyPlayer`, the
  reference baseline of the EAAI-2021 Gin Rummy AI challenge — take the
  face-up card only into an immediate meld, shed a uniformly random card
  among the minimal-deadwood choices (never repeating a draw/discard pair
  within a round), knock at the first legal opportunity.  It is a fixed
  measuring stick: win rates against it are comparable with the agents in
  the EAAI-21 literature.  The `arena` and `play` examples accept it as
  bot spec `eaai`, and `arena` gains a `--rules eaai` preset (modern
  bonuses, no big gin) matching the challenge's round conditions.
  Measured at seed 7 under those rules: the default heuristic wins 57.4%
  of games against the baseline (95% CI 53.0–61.7%, 500 games) while
  conceding single rounds at 39.9% by gin-hunting design; `mc:64` takes
  52.4% of 4000 rounds and 53.3% of 600 games; `mc:128` takes 54.0% of
  4000 rounds.  Published EAAI-21 entries sit around 55–68% against the
  same baseline, metrics varying by paper.
- A `parallel` cargo feature (off by default): Monte Carlo scoring batches
  spread their rollouts across the CPU cores via rayon.  Decisions are
  bit-identical to the serial build — batch results are collected in world
  order and reduced sequentially — so a seeded bot plays the same games,
  just sooner: on a 16-core machine a 64-sample decision runs about 3×
  faster.  The default build and the wasm front end are unaffected.

### Changed

- `MonteCarloBot` now rolls its sampled worlds in growing batches and drops
  a challenger action at a batch boundary once the greedy incumbent's paired
  advantage over it is statistically clear — the same two-standard-error bar
  a challenger must clear to be preferred — stopping the decision outright
  when no challenger remains.  Easy decisions cost a fraction of the sample
  budget while close ones still use every world, and a survivor's statistics
  are bit-identical to an unbatched run: over 4000 seeded arena rounds
  against the default greedy the decisions were literally identical, while
  whole rounds ran ~8% faster at mc:64 and ~25% faster at mc:128, the saving
  growing with the sample count.  A hint row the bot eliminated early
  reports its equity from the worlds it saw rather than the full sample
  count.

- `MonteCarloBot` samples each decision's worlds from one shuffled pool
  instead of rebuilding a fresh deck for every biased opponent draw.  The
  sampled distribution, measured strength, and per-decision latency are
  unchanged (whole-game throughput improves a few percent late in rounds,
  where the old rebuilds were largest), but the generator is consumed in a
  different order, so a seeded bot plays different — equally strong — games
  than it did in 0.1.3.

- The solver/hint panel's knock candidate is now labeled plainly `"knock"`
  instead of naming the discard (e.g. `"knock, drop 7♣"`).  Knocking always
  sheds the largest deadwood card, and the panel already lists only one
  knock row, so naming the card added nothing a player could act on.
- The browser `Hint` now shows a single instant read rather than sampling in
  the background and re-rendering as the equities sharpen.  The read comes
  straight from `MonteCarloBot::assess` at the bot's own sample count, which is
  plenty to rank the candidates; the live "worlds" counter and the deepening
  loop are gone.
- `MonteCarloBot` now weighs a single best-shed knock at a discard rather than
  one knock per candidate shed, so the move it plays comes from exactly the
  candidate set the solver read (`assess`) shows.  Knocking always sheds the
  largest deadwood, so the dropped knocks were dominated and never chosen;
  measured strength is unchanged (mc:64 still wins ≈63% of decisive rounds
  against the default greedy over 4000 seeded rounds), but seeded play can
  differ in the rare position where the old code would have knocked on a worse
  shed.

### Removed

- `MonteCarloBot::hint_open` and `hint_refine`.  They computed the same read as
  `assess` incrementally, so a caller could deepen it batch by batch; nothing
  outside the crate needed that, and `assess` alone covers both the terminal
  and browser hint.  Callers that were deepening a read should call `assess`
  with the sample count they want.
- `MonteCarloBot::max_candidates`.  The number of candidate discards a turn
  evaluates is now fixed at 4; no caller ever set it.
- `View::game_margin`.  Its value is `game_scores()[0] − game_scores()[1]`;
  no shipped strategy read it, and `game_scores` already exposes both totals
  (and the distance to `game_target` a lone margin cannot recover).  A caller
  that wants the lead can subtract the two totals.

## [0.1.3] - 2026-07-07

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
- `MonteCarloBot::hint_open` and `hint_refine` compute that same read
  incrementally: `hint_open` returns an instant estimate from a small batch of
  worlds and keeps them, and each `hint_refine` folds in more worlds to sharpen
  it without repeating the earlier rollouts.  Because the rollout is
  deterministic and worlds are the pairing unit, the deepened read is exact —
  identical to one `assess` over all the worlds combined — so a caller can
  spread the work across time instead of blocking on one long evaluation.

### Changed

- The hint's draw-phase labels now name the card on offer — `take 4♠`
  rather than a bare `take` or `take pile` — so a newcomer to the game or
  to English can tell which card the move takes.  The hidden stock draw
  stays a generic `draw stock`, since its card is not shown.
- `MonteCarloBot`'s assumed opponent hand now keeps improving for the
  whole round instead of leveling off about a third of the way in.  Its
  rollouts model the hidden hand as the best of several drawn hands, more
  of them the deeper the pile has grown; that scaling no longer stops
  increasing partway through, so late-round equity and EV reads no longer
  assume an opponent who stopped getting better early.
- The browser `Hint` now answers like an analysis engine.  It shows a read at
  once, then keeps sampling in the background and re-renders the same panel as
  the equities sharpen — with a live worlds count — over about a second and a
  half, until you move.  The instant first read is as before; it simply no
  longer stops there.
- The browser front end remembers your difficulty choice across visits
  instead of resetting to Medium on every reload.
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

[Unreleased]: https://github.com/jdh8/gin-rummy-engine/compare/0.2.0...HEAD
[0.2.0]: https://github.com/jdh8/gin-rummy-engine/compare/0.1.3...0.2.0
[0.1.3]: https://github.com/jdh8/gin-rummy-engine/compare/0.1.2...0.1.3
[0.1.2]: https://github.com/jdh8/gin-rummy-engine/compare/0.1.1...0.1.2
[0.1.1]: https://github.com/jdh8/gin-rummy-engine/compare/0.1.0...0.1.1
[0.1.0]: https://github.com/jdh8/gin-rummy-engine/releases/tag/0.1.0
