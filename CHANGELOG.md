# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `McConfig::hand_calibration` makes the Monte Carlo search model an
  opponent hand that has actually been played.  Sampled worlds used to
  keep the strongest of several uniform draws, which is a much weaker
  hand than any real opponent holds — at a twelve-card discard pile it
  carries about 28 deadwood where a hand played since the deal carries
  13 — so an undercut looked unlikely exactly where it is likeliest.
  With the knob on, a sampled hand is developed against the unseen cards
  until it reaches the deadwood measured for that point in the round.
  It is worth roughly three points of game win share against
  `EaaiSimpleBot` and two against `gold-paper`, costs about one against
  `marjj-v5-surrogate` — a realistic hand is worth more against
  opponents who knock than against one who plays for gin — and adds 2.5%
  to a 128-sample decision.  The default is off, because this crate's
  current strength work targets the camper; turn it on for general play
  against ordinary opponents.
- Public game-protocol primitives: `DealerRotation` distinguishes ordinary
  winner-deals-next play from EAAI's
  `AlternateAfterScoredRound`, `Table::dealer_rotation` passes that choice
  into every strategy `View`, and `eaai_rules()` returns the challenge's
  exact scoring preset (no Big Gin, boxes, game bonus, or shutout bonus).
  Score-aware Monte Carlo value tables are now selected by both rules and
  dealer rotation.
- The arena accepts `--seeds 7,8` for per-seed plus pooled runs and
  `--format json` for the versioned `gin-rummy-arena/v1` evidence schema,
  including raw scores, finish attribution, cluster moments, primary-test
  fields, and source/environment reproducibility metadata.  Exact sign-test
  p-values are retained in log space and include a canonical decimal string;
  the numeric field is `null`, never a false zero, below `f64` range.  Bare
  `mc` and `mca` now mean 128 samples; `mc:N` and `mca:N` remain explicit
  overrides.
- Benchmark-only strong-opponent adaptations and source-conformance checks
  cover the 2026 Gold Standard Agent paper policy (`gold-paper`) and a
  public MARJJ v5 host surrogate (`marjj-v5-surrogate`).  Gold's exactness
  is limited to meld decomposition, not full-game optimality, and the
  separately named MARJJ v5 source is not claimed to be the championship
  submission.  In the completed corrected-protocol panel, `greedy`, `mc:64`,
  and `mc:128` won 62.2%, 69.5%, and 74.5% of games against Gold, while
  winning 29.2%, 42.4%, and 46.7% against the MARJJ surrogate.  Both seeds
  agreed in direction for every matchup and all six pooled Holm-adjusted
  exact p-values were below .001.  The
  [strong-opponent report](docs/strong-opponents.md) and
  [raw JSON](docs/strong-opponents.json) preserve the full provenance,
  adaptations, intervals, scores, and round diagnostics.
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
  API; `samples(n)` remains as sugar.  Pre-correction measurements through
  the knobs — mc:64 against the challenge baseline, 3000 deal-paired games
  per arm and seed — historically found that modeling the baseline's
  conservative draw rule (`MeldOnly`) is no better than the default eager model
  (53.9% vs 54.8%), so belief fidelity is not monotone in strength, and
  that a nearly-gin-hunting own continuation (`rollout_knock_self: 2`)
  keeps a small repeatable edge over the knock-ASAP default on both
  seeds tried (55.2%/55.7% vs 54.8%/53.9%) while the middling thresholds
  4 and 8 are worse than either extreme.  That edge does not survive
  strong opposition — the same patient continuation scores 47.9% vs
  48.9% for the default against an unmodified mc:64 — so it is an
  exploit of the baseline's knock-ASAP habit, available as a knob for
  anyone targeting that baseline.  No finding cleared the bar to change
  a default.  Those rates remain tuning history rather than current EAAI
  evidence; the completed fixed panel below measures the current defaults.
- The `arena` example measures like an instrument now.  Trials are seeded
  by index and fan out across the CPUs.  Every trial defaults to a
  common-random-number *mirrored pair*: the bots swap seats on one cloned
  round deal or identically seeded whole-game shuffle streams
  (`--unpaired` restores independent trials).  A different dead/scored
  result can make the orientations' later dealer sequences diverge, so a
  game pair is correlated without being an exact replay throughout.
  Mirrored win rates now carry 95% pair-cluster intervals, and the primary
  comparison is an exact two-sided sign test over pairs swept by each bot;
  the paired-normal z statistic remains diagnostic only.
  `--alternate-dealer` follows the EAAI challenge: the dealer flips after
  each scored hand, while a dead hand is redealt by the same dealer.  A run
  without `--seed` or `--seeds` picks and prints a seed for reproduction.
- The `tune` example sweeps `MonteCarloBot` knobs: `--mc-samples N`
  makes the candidate a Monte Carlo bot and `--rollout-knock`,
  `--opp-knock`, `--opp-model`, `--gate`, `--max-candidates`, and
  `--opp-strength` sweep `McConfig` fields as comma lists (arms are the
  cartesian product).  The fixed opponent can now be the challenge
  baseline (`--opponent eaai`) and the rules preset `eaai` is accepted,
  so a knob can be tuned against the yardstick it aims at.
- `tune` also sweeps against the benchmark-only strong adaptations
  (`--opponent gold-paper`, `--opponent marjj-v5-surrogate`, spelled as
  the arena spells them) and follows the EAAI challenge dealer protocol
  under `--alternate-dealer`, so a knob sweep faces the same opponent and
  the same rotation as the published panels.  This matters beyond
  bookkeeping: the Monte Carlo value tables are keyed by rules *and*
  dealer rotation, so a sweep left on the default winner-deals-next rule
  selects arms against a different value function than the panel that
  will confirm them.  The summary line now names the protocol it played.
- `scripts/bench-panel.sh` regenerates README's benchmark table.  The
  published numbers were previously a hand-kept transcript of runs
  nobody else could repeat; the script pins the bots, the rules, the
  dealer protocol, the seeds and the counts, prints the table on stdout
  and the full arena log on stderr, and stamps the commit it ran at.
  The regenerated table now reports the corrected dead-hand dealer rule,
  exact EAAI preset, pair-cluster intervals, exact pair-sweep tests, and raw
  target-reaching scores.  Against `EaaiSimpleBot`, `greedy`, `mc:64`, and
  `mc:128` win 59.8%, 65.4%, and 69.2% of games respectively; every exact
  pair-sweep p-value is below .001.

### Changed

- `MonteCarloBot` now defaults to patient own continuations
  (`rollout_knock_self: 0`) and draws twice as many candidate opponent
  hands (`opponent_strength_percent: 200`).  The search therefore compares
  knocking now with building toward gin instead of ending every modeled
  line at the first legal knock, while pricing a developed opponent less
  optimistically.  In the fixed corrected-protocol panels, `mc:128` wins
  69.2% (68.3–70.2%) of games against `EaaiSimpleBot`, 74.5%
  (73.8–75.2%) against `gold-paper`, and 46.7% (45.9–47.5%) against
  `marjj-v5-surrogate`; the latter remains an opponent edge, so the parity
  target is not reached.  A hard 128-sample decision rises from 24.4 to
  28.2 ms (+15.2%) on an idle machine, and hand calibration adds another
  2.5% when enabled.
- `EaaiSimpleBot`'s documentation now states what the bot is and how it
  differs from the challenge framework's Java player.  It is an
  implementation of that player's published policy rather than a
  transliteration of its code, and the four places it departs are now
  listed with their consequences — including two that were previously
  unrecorded: the layoff sweep, where this bot's greedy layoff can only
  do better for the defender than the framework's first-fit pass, so win
  rates published against this baseline are conservative rather than
  inflated; and the round's (draw, discard) loop breaker, which keys on
  an ordered pair where the original keys on an unordered one.  The same
  source audit corrected two host-protocol mismatches: `--rules eaai` now
  has no Big Gin, box, game, or shutout bonus, and
  `--alternate-dealer` flips only after a scored hand while a dead hand
  retains the dealer.  Bot-policy behavior is otherwise unchanged.  The
  corrected fixed panel now supersedes the earlier EAAI rates and intervals:
  `greedy`, `mc:64`, and `mc:128` win 59.8%, 65.4%, and 69.2% of games,
  respectively, against the baseline.
- `MonteCarloBot` now values a decision by its probability of winning the
  **game**, not by the round points it banks.  Short of a clinch the old
  equity was affine in round points, so the search had no sense of the
  scoreboard: a point of a commanding lead counted for exactly as much as
  a point of a hopeless deficit, and the bot neither banked a lead nor
  pressed a comeback.  It now prices a round outcome through a solved
  win-probability function of both scores and the dealer rotation, so it
  plays the lead it has.  Historical pre-correction measurements against
  the previous behavior used 12 000 deal-paired games per seed under what
  was then believed to be the EAAI challenge protocol.  They reported the
  new default at 51.1% and 51.4% of games on two seeds (+2.1 and +2.7
  points, p = 0.044 and p = 0.009), with 54.9% and 54.7% against
  `EaaiSimpleBot` where the old equity scored 54.8% and 53.9%, and 53.4%
  and 52.6% against `HeuristicBot` where it scored 52.9%.  Those figures
  require a corrected-protocol rerun and are retained only as development
  history.  `McConfig::game_value` selects the behavior —
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
  stream depends only on `--seed` and the trial index.  An unchanged
  build reproduces its run; old-vs-new comparisons receive the same
  indexed random streams, but changed play can alter dead/scored results
  and therefore later dealer assignments, so whole games are
  common-random-number comparisons rather than exact deal-for-deal
  replays.  A seeded 0.2.0 arena run is not reproduced by this version.
  `--rounds N`/`--games N` now count trials (mirrored pairs by default, so
  twice as many rounds or games are played).
- The fixed README panel now measures the corrected EAAI protocol with
  mirrored-pair clusters and exact pair-sweep sign tests.  Against
  `EaaiSimpleBot`, `greedy` wins 39.4% of decisive rounds but 59.8%
  (59.0–60.6%) of games, scoring 90.28–78.76 raw points/game; `mc:64` wins
  46.1% of rounds and 65.4% (64.6–66.2%) of games, scoring 92.52–70.67;
  `mc:128` wins 47.1% of rounds and 69.2% (68.3–70.2%) of games, scoring
  94.93–66.87.  `mc:64` beats `greedy` head-to-head in 63.6%
  (62.8–64.4%) of 12,000 games, 92.94–71.62 raw score/game.  Every exact
  pair-sweep p-value is below .001.

### Fixed

- Arena JSON now records `McConfig::hand_calibration`, and the strong-panel
  validator recognizes the promoted patient-rollout defaults.  Full-panel
  runs can therefore publish complete configuration provenance instead of
  failing only after every measured leg has finished.

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
  measuring stick intended for comparison with the agents in the EAAI-21
  literature.  The `arena` and `play` examples accept it as
  bot spec `eaai`, and `arena` gains a `--rules eaai` preset (modern
  bonuses, no big gin), then believed to match the challenge's conditions.
  Measured at seed 7 under those rules: the default heuristic wins 57.4%
  of games against the baseline (95% CI 53.0–61.7%, 500 games) while
  conceding single rounds at 39.9% by gin-hunting design; `mc:64` takes
  52.4% of 4000 rounds and 53.3% of 600 games; `mc:128` takes 54.0% of
  4000 rounds.  Published EAAI-21 entries sit around 55–68% against the
  same baseline, metrics varying by paper.  A later source audit found
  that these release-time rates used extra game-level bonuses and not the
  corrected scored-hand-only dealer protocol; they are historical and are
  not directly comparable with corrected-protocol results.
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
