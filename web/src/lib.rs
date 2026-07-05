//! Browser front end for gin-rummy-engine.
//!
//! The whole game already lives in the engine — [`Table`] drives it and
//! validates every action — so this crate only replaces the terminal I/O of
//! `examples/play.rs` with a JSON snapshot plus one method per human decision.
//! The human seat is a one-shot [`Strategy`] ([`Pending`]): the UI sets the
//! action for the current decision point and the driver is stepped once, so a
//! browser can collect each move as a click without blocking.
//!
//! Everything decision-shaped (narration, describing results, the knock shed)
//! is lifted almost verbatim from `examples/play.rs`; only the output target
//! changed from `println!` to a `Snapshot` and a running log.

use gin_rummy::{Card, Game, Hand, Melds, Phase, Player, RoundResult, Rules, best_melds, deadwood};
use gin_rummy_engine::{
    DrawAction, HeuristicBot, HeuristicConfig, Layoff, MonteCarloBot, Strategy, Table, TurnAction,
    UpcardAction, View,
};
use rand::rngs::StdRng;
use rand::{RngExt as _, SeedableRng};
use serde::Serialize;
use wasm_bindgen::prelude::*;

const HUMAN: Player = Player::One;
const BOT: Player = Player::Two;

// ---------------------------------------------------------------------------
// The human seat as a one-shot strategy
// ---------------------------------------------------------------------------

/// One human decision, staged by the UI before the driver is stepped.
enum HumanAction {
    Upcard(UpcardAction),
    Draw(DrawAction),
    Turn(TurnAction),
}

/// The human seat: returns whatever action the UI staged for the current
/// phase.  Layoffs are not a human choice here — they auto-resolve with the
/// greedy bot, exactly as `examples/play.rs` does.
#[derive(Default)]
struct Pending {
    action: Option<HumanAction>,
}

impl Strategy for Pending {
    fn offer_upcard(&mut self, _view: &View<'_>) -> UpcardAction {
        match self.action.take() {
            Some(HumanAction::Upcard(a)) => a,
            _ => unreachable!("upcard step without a staged upcard action"),
        }
    }

    fn choose_draw(&mut self, _view: &View<'_>) -> DrawAction {
        match self.action.take() {
            Some(HumanAction::Draw(a)) => a,
            _ => unreachable!("draw step without a staged draw action"),
        }
    }

    fn play_turn(&mut self, _view: &View<'_>) -> TurnAction {
        match self.action.take() {
            Some(HumanAction::Turn(a)) => a,
            _ => unreachable!("turn step without a staged turn action"),
        }
    }

    fn choose_layoff(&mut self, view: &View<'_>) -> Option<Layoff> {
        // ponytail: reuse the bot's greedy layoff; the human need not choose.
        HeuristicBot::new().choose_layoff(view)
    }

    fn name(&self) -> &str {
        "you"
    }
}

/// What the driver should do at the human's seat right now.
enum HumanStep {
    /// A genuine choice — stop and wait for the UI.
    WaitInput,
    /// No choice (forced stock draw, or a layoff): step with [`Pending`].
    AutoStep,
    /// All eleven cards meld: declare big gin, which is strictly dominant.
    BigGin(Melds),
}

// ---------------------------------------------------------------------------
// The game, driven one decision at a time
// ---------------------------------------------------------------------------

/// A whole game in progress: the driver, the bot, and the human's staged
/// action, plus the running transcript.  Split out from the wasm wrapper so it
/// can be driven directly in a native test.
struct Core {
    game: Game,
    table: Table,
    bot: Box<dyn Strategy>,
    pending: Pending,
    rng: StdRng,
    round_no: u32,
    log: Vec<String>,
    last_result: Option<RoundResult>,
    last_move: Option<Move>,
    settled: Option<gin_rummy::FinalScore>,
    /// A round finished and the game continues: the showdown is on screen and
    /// we wait for the player's Continue click before dealing the next round.
    awaiting_continue: bool,
}

impl Core {
    /// Deal the first round and run any leading bot actions up to the human's
    /// first decision.
    fn new(bot_spec: &str, rules: Rules, seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        // Mirror `play.rs::make_bot` ordering so a shared seed reproduces the
        // terminal game: the mc bot consumes one draw from `rng` before the deal.
        let bot = make_bot(bot_spec, &mut rng);
        let game = Game::new(rules, HUMAN);
        let dealer = game.next_dealer();
        let scores = [game.score(Player::One), game.score(Player::Two)];
        let table = Table::new(game.deal(&mut rng)).scores(scores);
        // The UI paces the game from the deal by ticking `step_once`, so unlike
        // `play.rs` we do not drain to the first human decision here.
        Self {
            game,
            table,
            bot,
            pending: Pending::default(),
            rng,
            round_no: 1,
            log: vec![round_header(1, dealer)],
            last_result: None,
            last_move: None,
            settled: None,
            awaiting_continue: false,
        }
    }

    // --- human decisions -------------------------------------------------

    fn take_upcard(&mut self) {
        self.human(HumanAction::Upcard(UpcardAction::Take));
    }
    fn pass_upcard(&mut self) {
        self.human(HumanAction::Upcard(UpcardAction::Pass));
    }
    fn draw_stock(&mut self) {
        self.human(HumanAction::Draw(DrawAction::Stock));
    }
    fn take_discard(&mut self) {
        self.human(HumanAction::Draw(DrawAction::TakeDiscard));
    }
    fn discard(&mut self, card: Card) {
        self.human(HumanAction::Turn(TurnAction::Discard(card)));
    }

    /// Knock on the smallest deadwood, shedding the best card — the shed is
    /// forced (face down), so it is not a human choice.
    fn knock(&mut self) {
        let (discard, melds) = {
            let view = self.table.view(HUMAN);
            let discard = best_shed(view.hand(), view.taken_discard());
            (discard, best_melds(view.hand() - discard.into()))
        };
        self.human(HumanAction::Turn(TurnAction::Knock { discard, melds }));
    }

    /// Stage a human action and step the driver once.  The bot's reply is *not*
    /// run here: the UI ticks [`Core::step_once`] itself, pacing and animating
    /// each following action.
    fn human(&mut self, action: HumanAction) {
        let before_phase = self.table.round().phase();
        let before_top = self.table.round().discard_pile().last().copied();
        let before_pile = self.table.round().discard_pile().len();
        let before_hand = self.table.view(HUMAN).hand();
        self.last_move = None;
        self.pending.action = Some(action);
        if let Err(e) = self.table.step(&mut self.pending) {
            // A well-behaved UI only offers legal moves; surface anything else.
            self.log.push(format!("Illegal: {e}"));
            return;
        }
        self.last_move = self.move_for("you", before_phase, before_top, before_pile, before_hand);
    }

    /// Advance the game by at most one visible action: one bot move, one of the
    /// human's forced/auto steps, or finishing a round (and dealing the next).
    /// Does nothing once the human has a real decision or the game is settled —
    /// the UI paces calls to this, animating the move each one reports.
    fn step_once(&mut self) {
        let Some(seat) = self.table.turn() else {
            self.last_move = None; // a deal, or the settled game, is not a move
            // Record a finished round once, then hold: the UI keeps the
            // showdown (both hands, the knock, any layoffs) on screen and deals
            // the next round only when the player clicks Continue.
            if self.settled.is_none() && !self.awaiting_continue {
                self.record_round();
            }
            return;
        };

        // Pre-step public state, for narration and the move signal.
        let before_phase = self.table.round().phase();
        let before_top = self.table.round().discard_pile().last().copied();
        let before_pile = self.table.round().discard_pile().len();
        let before_hand = self.table.view(HUMAN).hand();

        if seat == HUMAN {
            match self.human_step() {
                HumanStep::WaitInput => return, // a real choice: leave it to the UI
                HumanStep::AutoStep => {
                    let _ = self.table.step(&mut self.pending);
                }
                HumanStep::BigGin(melds) => {
                    self.log.push("You declare BIG GIN!".into());
                    self.pending.action = Some(HumanAction::Turn(TurnAction::BigGin(melds)));
                    let _ = self.table.step(&mut self.pending);
                }
            }
            self.last_move =
                self.move_for("you", before_phase, before_top, before_pile, before_hand);
        } else {
            self.table
                .step(&mut *self.bot)
                .expect("the bot always chooses legal actions");
            self.narrate_bot(before_phase, before_top, before_pile);
            self.last_move =
                self.move_for("bot", before_phase, before_top, before_pile, before_hand);
        }
    }

    /// What the driver should do at the human's seat right now.
    fn human_step(&self) -> HumanStep {
        let view = self.table.view(HUMAN);
        match view.phase() {
            // The forced stock draw after both pass consults no one.
            Phase::Draw if !view.can_take_discard() => HumanStep::AutoStep,
            Phase::Layoff => HumanStep::AutoStep,
            Phase::Discard if view.deadwood() == 0 && view.rules().big_gin_bonus.is_some() => {
                HumanStep::BigGin(view.best_melds())
            }
            Phase::Upcard | Phase::Draw | Phase::Discard => HumanStep::WaitInput,
            Phase::Finished => HumanStep::AutoStep,
        }
    }

    /// Whether the human has a genuine choice pending, as opposed to a forced
    /// step the driver resolves itself.  Drives `Snapshot::your_turn`, so the UI
    /// keeps ticking through forced draws, layoffs, and big gin.
    fn awaiting_human_input(&self) -> bool {
        self.settled.is_none()
            && self.table.turn() == Some(HUMAN)
            && matches!(self.human_step(), HumanStep::WaitInput)
    }

    /// The move just applied, described from legally-visible information, for the
    /// UI to animate.  A card drawn from the stock is named only for the human's
    /// own draw (a hand diff); the bot's stock draw stays hidden (invariant 1).
    fn move_for(
        &self,
        actor: &'static str,
        before_phase: Phase,
        before_top: Option<Card>,
        before_pile: usize,
        before_hand: Hand,
    ) -> Option<Move> {
        let round = self.table.round();
        let mv = |kind, card| Move { actor, kind, card };
        match before_phase {
            Phase::Upcard => Some(if round.phase() == Phase::Discard {
                mv("take", before_top.map(|c| c.to_string()))
            } else {
                mv("pass", None)
            }),
            Phase::Draw => Some(if round.discard_pile().len() < before_pile {
                mv("take", before_top.map(|c| c.to_string()))
            } else {
                // Drawn from the stock: name it only for the human's own hand.
                let card = (actor == "you")
                    .then(|| (self.table.view(HUMAN).hand() - before_hand).iter().next())
                    .flatten()
                    .map(|c| c.to_string());
                mv("draw_stock", card)
            }),
            Phase::Discard => Some(match round.result() {
                Some(RoundResult::BigGin { .. }) => mv("big_gin", None),
                // A normal discard, a knock, or gin all shed a visible top card.
                _ => mv(
                    "discard",
                    round.discard_pile().last().map(|c| c.to_string()),
                ),
            }),
            Phase::Layoff => Some(mv("layoff", None)),
            Phase::Finished => None,
        }
    }

    /// Record the finished round, then either settle the game or hold for the
    /// player's Continue click.  Dealing the next round is deferred to
    /// [`Core::next_round`] so the showdown stays on screen — knocking,
    /// spreading melds, and layoffs all decide the score and deserve a look.
    fn record_round(&mut self) {
        let result = self
            .table
            .round()
            .result()
            .expect("a turnless round finished");
        self.last_result = Some(result);
        self.game
            .record(result)
            .expect("a result from the round it was dealt for records cleanly");
        self.log.push(format!(
            "Round {}: {}.",
            self.round_no,
            describe(result, self.game.rules())
        ));
        self.log.push(format!(
            "Score — you {} : {} bot.",
            self.game.score(HUMAN),
            self.game.score(BOT),
        ));
        if self.game.is_over() {
            let settled = self
                .game
                .final_score()
                .expect("a game that is over settles");
            self.log.push(final_line(&settled));
            self.settled = Some(settled);
        } else {
            self.awaiting_continue = true;
        }
    }

    /// Clear the showdown and deal the next round.  A no-op unless a finished
    /// round is waiting on the player's Continue click.
    fn next_round(&mut self) {
        if !self.awaiting_continue {
            return;
        }
        self.awaiting_continue = false;
        self.last_move = None;
        self.last_result = None;
        self.round_no += 1;
        let dealer = self.game.next_dealer();
        let scores = [self.game.score(Player::One), self.game.score(Player::Two)];
        self.table = Table::new(self.game.deal(&mut self.rng)).scores(scores);
        self.log.push(round_header(self.round_no, dealer));
    }

    /// Narrate what the bot just did from public information alone — a direct
    /// port of `play.rs::narrate_bot` that appends to the log.
    fn narrate_bot(&mut self, before_phase: Phase, before_top: Option<Card>, before_pile: usize) {
        match before_phase {
            Phase::Upcard => {
                let msg = if self.table.round().phase() == Phase::Discard {
                    format!(
                        "Bot takes the {}.",
                        before_top.expect("a take implies a pile top")
                    )
                } else {
                    "Bot passes.".into()
                };
                self.log.push(msg);
            }
            Phase::Draw => {
                let msg = if self.table.round().discard_pile().len() < before_pile {
                    format!(
                        "Bot takes the {}.",
                        before_top.expect("a take implies a pile top")
                    )
                } else {
                    "Bot draws from the stock.".into()
                };
                self.log.push(msg);
            }
            Phase::Discard => {
                let round = self.table.round();
                let top = round.discard_pile().last().copied();
                match round.result() {
                    Some(RoundResult::BigGin { .. }) => {
                        self.log.push("Bot declares BIG GIN!".into())
                    }
                    Some(RoundResult::Gin { .. }) => self.log.push(format!(
                        "Bot discards {} and goes gin!",
                        top.expect("gin sheds a discard"),
                    )),
                    _ if round.knocker() == Some(BOT) => {
                        self.log.push(format!(
                            "Bot discards {} and knocks.",
                            top.expect("a knock sheds a discard"),
                        ));
                        let spread = self
                            .table
                            .view(HUMAN)
                            .spread()
                            .map(|meld| meld.to_string())
                            .collect::<Vec<_>>()
                            .join(" ");
                        self.log.push(format!("Bot spreads: {spread}"));
                    }
                    _ => self.log.push(format!(
                        "Bot discards {}.",
                        top.expect("a turn ends with a discard"),
                    )),
                }
            }
            Phase::Layoff => {
                let round = self.table.round();
                let laid = round.laid_off();
                let msg = if round.phase() == Phase::Layoff {
                    format!("Bot lays off (so far: {}).", list(laid))
                } else if laid.is_empty() {
                    "Bot lays nothing off.".into()
                } else {
                    format!("Bot lays off {}.", list(laid))
                };
                self.log.push(msg);
            }
            Phase::Finished => {}
        }
    }

    /// The full legally-visible position, as the UI renders it.
    fn snapshot(&self) -> Snapshot {
        let over = self.settled.is_some();
        let view = self.table.view(HUMAN);
        let taken = view.taken_discard();
        let arranged = view.best_melds();
        // At a knock/gin showdown both hands go face up to be scored; reveal
        // the bot's melds, its deadwood, and any laid-off cards.  A dead hand
        // has no knocker and stays concealed.
        let round = self.table.round();
        let bot = (round.knocker().is_some()).then(|| best_melds(round.hand(BOT)));
        Snapshot {
            round_no: self.round_no,
            phase: phase_name(view.phase()),
            your_turn: self.awaiting_human_input(),
            can_knock: view.phase() == Phase::Discard
                && knock_deadwood(view.hand(), taken) <= view.knock_limit(),
            melds: arranged
                .iter()
                .map(|meld| cards_by_suit(meld.cards(), taken))
                .collect(),
            loose: cards_by_rank(arranged.deadwood_cards(), taken),
            deadwood: view.deadwood(),
            upcard: view.upcard().map(|c| card_json(c, None)),
            pile_len: view.discard_pile().len(),
            stock_len: view.stock_len(),
            bot_hand_len: view.opponent_hand_len(),
            opponent_known: cards_by_rank(view.opponent_known(), None),
            taken_discard: taken.map(|c| c.to_string()),
            you_score: self.game.score(HUMAN),
            bot_score: self.game.score(BOT),
            round_over: self.awaiting_continue,
            result: self.last_result.map(|r| describe(r, self.game.rules())),
            bot_melds: bot
                .iter()
                .flat_map(|melds| melds.iter().map(|meld| cards_by_suit(meld.cards(), None)))
                .collect(),
            bot_loose: bot
                .as_ref()
                .map(|melds| cards_by_rank(melds.deadwood_cards(), None))
                .unwrap_or_default(),
            laid_off: if bot.is_some() {
                cards_by_rank(round.laid_off(), None)
            } else {
                Vec::new()
            },
            game_over: over,
            winner: self
                .settled
                .as_ref()
                .map(|s| if s.winner == HUMAN { "you" } else { "bot" }),
            last_move: self.last_move.clone(),
            log: self.log.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot: the JSON the browser renders
// ---------------------------------------------------------------------------

/// One card, ready to render and to echo back as `code` for a discard.
#[derive(Serialize)]
struct CardJson {
    /// The card's canonical name, e.g. `"T♠"`, parseable back into a `Card`.
    code: String,
    /// Rank as 1–13 (ace low), for a friendly label.
    rank: u8,
    /// Suit letter `C`/`D`/`H`/`S`, for colour and glyph.
    suit: char,
    /// The just-taken card, which may not be shed this turn.
    taken: bool,
}

/// The action just applied, for the UI to animate a single card between zones.
#[derive(Serialize, Clone)]
struct Move {
    /// `"you"` or `"bot"` — whose card moved (and thus its hand zone).
    actor: &'static str,
    /// `draw_stock` | `take` | `discard` | `big_gin` | `pass` | `layoff`.
    kind: &'static str,
    /// The moving card's `code`, or `None` when it is face down (the bot's
    /// hidden stock draw) or there is no card (pass/layoff/big gin).
    card: Option<String>,
}

/// Everything one seat may legally see, plus the running transcript.
#[derive(Serialize)]
struct Snapshot {
    round_no: u32,
    phase: &'static str,
    your_turn: bool,
    /// Whether knocking is legal right now (deadwood within the limit).
    can_knock: bool,
    /// The best meld arrangement, one inner list per meld.
    melds: Vec<Vec<CardJson>>,
    /// Loose deadwood, ordered by rank.
    loose: Vec<CardJson>,
    deadwood: u8,
    upcard: Option<CardJson>,
    pile_len: usize,
    stock_len: usize,
    /// The bot's hand size, for the face-down opponent fan.
    bot_hand_len: usize,
    opponent_known: Vec<CardJson>,
    /// `code` of the card that may not be shed this turn, if any.
    taken_discard: Option<String>,
    you_score: u16,
    bot_score: u16,
    /// A round just finished and the game continues; the UI shows Continue.
    round_over: bool,
    /// The finished round's result summary, for the between-rounds banner.
    result: Option<String>,
    /// The bot's melds, laid face up at the showdown (empty until a knock).
    bot_melds: Vec<Vec<CardJson>>,
    /// The bot's remaining deadwood at the showdown.
    bot_loose: Vec<CardJson>,
    /// Cards the defender laid off onto the knocker's spread, shown apart.
    laid_off: Vec<CardJson>,
    game_over: bool,
    winner: Option<&'static str>,
    /// The move that produced this snapshot, if any, for animation.
    last_move: Option<Move>,
    log: Vec<String>,
}

// ---------------------------------------------------------------------------
// The wasm-bindgen wrapper
// ---------------------------------------------------------------------------

/// A gin rummy game the browser drives one decision at a time.  Each method
/// applies a move and returns the fresh [`Snapshot`] as a JSON string.
#[wasm_bindgen]
pub struct WebGame {
    core: Core,
}

#[wasm_bindgen]
impl WebGame {
    /// Start a game.  `bot` is `newbie`, `greedy`, or `mc[:samples]`, `rules` is
    /// `modern`/`classic`/`palace`, and `seed` is a decimal string (a shared
    /// seed reproduces the terminal `play` example exactly).
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(bot: &str, rules: &str, seed: &str) -> Self {
        Self {
            core: Core::new(bot, parse_rules(rules), seed.parse().unwrap_or(0)),
        }
    }

    /// The current position as JSON.
    #[must_use]
    pub fn snapshot(&self) -> String {
        json(&self.core.snapshot())
    }

    /// Advance one bot move or forced step and return the fresh snapshot.  The
    /// page calls this on a timer while `your_turn` is false, animating each
    /// reported `last_move` between the calls.
    pub fn tick(&mut self) -> String {
        self.core.step_once();
        self.snapshot()
    }

    /// Take the offered upcard.
    pub fn take_upcard(&mut self) -> String {
        self.core.take_upcard();
        self.snapshot()
    }

    /// Pass the offered upcard.
    pub fn pass(&mut self) -> String {
        self.core.pass_upcard();
        self.snapshot()
    }

    /// Draw the hidden top of the stock.
    pub fn draw_stock(&mut self) -> String {
        self.core.draw_stock();
        self.snapshot()
    }

    /// Take the top of the discard pile.
    pub fn take_discard(&mut self) -> String {
        self.core.take_discard();
        self.snapshot()
    }

    /// Discard the card named by `code` (a value from a `CardJson.code`).
    pub fn discard(&mut self, code: &str) -> String {
        if let Ok(card) = code.parse::<Card>() {
            self.core.discard(card);
        }
        self.snapshot()
    }

    /// Knock on the smallest deadwood.
    pub fn knock(&mut self) -> String {
        self.core.knock();
        self.snapshot()
    }

    /// Deal the next round after the between-rounds pause.
    pub fn next_round(&mut self) -> String {
        self.core.next_round();
        self.snapshot()
    }
}

// ---------------------------------------------------------------------------
// Free helpers (mostly lifted from examples/play.rs)
// ---------------------------------------------------------------------------

/// Build the bot named by `spec`, mirroring `play.rs::make_bot` so a shared
/// seed reproduces the terminal game.
fn make_bot(spec: &str, rng: &mut StdRng) -> Box<dyn Strategy> {
    let (kind, samples) = match spec.split_once(':') {
        Some((kind, n)) => (kind, n.parse::<u32>().ok()),
        None => (spec, None),
    };
    match kind {
        // A newcomer: knocks at the first legal chance, and is blind both to
        // the game score and to what a discard hands the opponent.
        "newbie" => {
            let mut config = HeuristicConfig::default();
            config.knock_threshold = 10;
            config.safety_weight = 0;
            config.score_awareness = 0;
            Box::new(HeuristicBot::with_config(config))
        }
        "greedy" => Box::new(HeuristicBot::new()),
        // Default to the Monte Carlo bot; it needs its own seeded generator.
        _ => Box::new(
            MonteCarloBot::new(StdRng::seed_from_u64(rng.random())).samples(samples.unwrap_or(64)),
        ),
    }
}

fn parse_rules(name: &str) -> Rules {
    match name {
        "classic" => Rules::classic(),
        "palace" => Rules::palace(),
        _ => Rules::new(),
    }
}

/// The largest-pip deadwood card, skipping the just-taken card — the card to
/// auto-knock on.  Mirrors the crate-private greedy `best_shed`.
fn best_shed(hand: Hand, taken: Option<Card>) -> Card {
    hand.iter()
        .filter(|&c| Some(c) != taken)
        .min_by_key(|&c| (deadwood(hand - c.into()), u8::MAX - c.rank.deadwood()))
        .expect("a full hand always has a legal discard")
}

/// The deadwood a knock would actually spread: the best arrangement *after*
/// shedding the best card.  `Round::knock` validates this ten-card figure
/// against the limit, not the eleven-card hand still holding the shed card, so
/// this — not `view.deadwood()` — is the honest gate for the Knock button.
fn knock_deadwood(hand: Hand, taken: Option<Card>) -> u8 {
    deadwood(hand - best_shed(hand, taken).into())
}

/// Narrate a finished round.  Bonus'd results spell out the score as
/// `earned + bonus = total` — the opponent's deadwood (or the undercut margin)
/// plus the fixed bonus — so the printed number matches the score change.  The
/// bonus is `points − earned` rather than a rules field so it can never drift
/// from the real pricing.
fn describe(result: RoundResult, rules: &Rules) -> String {
    let pts = result.points(rules);
    match result {
        RoundResult::Dead => "dead hand, nobody scores".into(),
        RoundResult::Knock { winner, margin } => format!("{} knock for {margin}", who(winner)),
        RoundResult::Undercut { winner, margin } => {
            format!(
                "{} undercut ({margin} + {} = {pts})",
                who(winner),
                pts - u16::from(margin)
            )
        }
        RoundResult::Gin { winner, deadwood } => {
            format!(
                "{} gin ({deadwood} + {} = {pts})",
                who(winner),
                pts - u16::from(deadwood)
            )
        }
        RoundResult::BigGin { winner, deadwood } => {
            format!(
                "{} BIG gin ({deadwood} + {} = {pts})",
                who(winner),
                pts - u16::from(deadwood)
            )
        }
        _ => format!("{result:?}"),
    }
}

fn final_line(settled: &gin_rummy::FinalScore) -> String {
    format!(
        "{} the game, {} : {}{}",
        if settled.winner == HUMAN {
            "You win"
        } else {
            "Bot wins"
        },
        settled.totals[settled.winner as usize],
        settled.totals[settled.winner.opponent() as usize],
        if settled.shutout {
            " — a shutout!"
        } else {
            ""
        },
    )
}

fn round_header(number: u32, dealer: Player) -> String {
    format!(
        "=== Round {number} ({} deal) ===",
        if dealer == HUMAN { "your" } else { "bot's" },
    )
}

fn who(player: Player) -> &'static str {
    if player == HUMAN { "You" } else { "Bot" }
}

fn list(cards: Hand) -> String {
    cards
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Upcard => "upcard",
        Phase::Draw => "draw",
        Phase::Discard => "discard",
        Phase::Layoff => "layoff",
        Phase::Finished => "finished",
    }
}

fn card_json(card: Card, taken: Option<Card>) -> CardJson {
    CardJson {
        code: card.to_string(),
        rank: card.rank.get(),
        suit: card.suit.letter(),
        taken: Some(card) == taken,
    }
}

/// Cards in the crate's native suit-major order (used within a meld).
fn cards_by_suit(hand: Hand, taken: Option<Card>) -> Vec<CardJson> {
    hand.iter().map(|c| card_json(c, taken)).collect()
}

/// Cards ordered by rank then suit — how most players scan loose deadwood.
fn cards_by_rank(hand: Hand, taken: Option<Card>) -> Vec<CardJson> {
    let mut cards: Vec<Card> = hand.iter().collect();
    cards.sort_by_key(|c| (c.rank, c.suit));
    cards.into_iter().map(|c| card_json(c, taken)).collect()
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("a snapshot serializes")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The Knock gate must judge the ten-card spread, not the eleven-card
    /// hand: ♣A♣2♣4♣5 ♦A♦4 ♥A♥3♥4 ♠A♠2 is deadwood 12 as it stands, but
    /// shedding the ♣5 leaves 7 — a legal knock the button must offer.
    #[test]
    fn knock_gate_ignores_the_card_to_be_shed() {
        let hand: Hand = "A245.A4.A34.A2".parse().expect("a legal eleven-card hand");
        assert_eq!(deadwood(hand), 12, "the hand still holding the shed card");
        assert_eq!(knock_deadwood(hand, None), 7, "after shedding the ♣5");
        assert!(knock_deadwood(hand, None) <= Rules::new().knock_limit);
    }

    /// Drive a whole game to completion through the public decision methods,
    /// exercising the `Pending`-strategy loop, the auto steps, and the round
    /// hand-off without any browser.
    #[test]
    fn a_scripted_game_reaches_a_settled_score() {
        let mut core = Core::new("greedy", Rules::new(), 42);
        let mut guard = 0;
        loop {
            // Tick through the bot's replies and the human's forced/auto steps,
            // exactly as the paced UI does, until a real decision or the end.
            while !core.awaiting_human_input() && core.settled.is_none() && !core.awaiting_continue
            {
                guard += 1;
                assert!(guard < 100_000, "the game must terminate");
                core.step_once();
            }
            if core.settled.is_some() {
                break;
            }
            // Between rounds the UI holds for a Continue click; drive it here.
            if core.awaiting_continue {
                // The showdown snapshot: round over, game not, a result to show,
                // and the bot's hand revealed exactly when someone knocked.
                let showdown = core.snapshot();
                assert!(showdown.round_over && !showdown.game_over && !showdown.your_turn);
                assert!(showdown.result.is_some(), "a finished round has a summary");
                let revealed = !showdown.bot_melds.is_empty() || !showdown.bot_loose.is_empty();
                assert_eq!(
                    revealed,
                    core.table.round().knocker().is_some(),
                    "the bot's hand is face up at a knock and only there",
                );
                core.next_round();
                let dealt = core.snapshot();
                assert!(!dealt.round_over && dealt.result.is_none() && dealt.bot_melds.is_empty());
                continue;
            }
            let snap = core.snapshot();
            assert!(snap.your_turn, "awaiting_human_input implies your_turn");
            match snap.phase {
                "upcard" => core.pass_upcard(),
                "draw" => core.draw_stock(),
                // Knock when the snapshot says it is legal — exercises knock()
                // and the bot's defending layoff; otherwise shed the best card.
                "discard" if snap.can_knock => core.knock(),
                "discard" => {
                    let card = {
                        let view = core.table.view(HUMAN);
                        best_shed(view.hand(), view.taken_discard())
                    };
                    core.discard(card);
                }
                other => panic!("unexpected phase for a human decision: {other}"),
            }
        }
        let settled = core.settled.expect("the loop only exits once settled");
        assert!(
            core.game.score(settled.winner) > 0,
            "the winner reached the game target",
        );
        assert!(core.snapshot().game_over);
    }

    /// Big gin is too rare to stumble on by hand, so it is pinned to a seed:
    /// seed 381's greedy self-play has the bot draw into a fully-melded hand.
    /// The whole web path must fire — the engine's `BigGin` result, the
    /// `big_gin` move the UI animates, the narrated log line, and a summary
    /// that names it — so a regression anywhere along it trips this test.
    #[test]
    fn big_gin_travels_the_whole_web_path() {
        let mut core = Core::new("greedy", Rules::new(), 381);
        let mut result = None;
        let mut saw_move = false;
        let mut guard = 0;
        while core.settled.is_none() {
            guard += 1;
            assert!(guard < 100_000, "the game must terminate");
            while !core.awaiting_human_input() && core.settled.is_none() && !core.awaiting_continue
            {
                core.step_once();
                saw_move |= matches!(
                    core.last_move,
                    Some(Move {
                        kind: "big_gin",
                        ..
                    })
                );
                if matches!(core.last_result, Some(RoundResult::BigGin { .. })) {
                    result = core.last_result;
                }
            }
            if core.settled.is_some() {
                break;
            }
            // Between rounds the UI holds for a Continue click; drive it here.
            if core.awaiting_continue {
                core.next_round();
                continue;
            }
            // The human plays greedily, knocking as soon as it is legal.
            let snap = core.snapshot();
            match snap.phase {
                "upcard" => core.pass_upcard(),
                "draw" => core.draw_stock(),
                "discard" if snap.can_knock => core.knock(),
                "discard" => {
                    let card = {
                        let view = core.table.view(HUMAN);
                        best_shed(view.hand(), view.taken_discard())
                    };
                    core.discard(card);
                }
                other => panic!("unexpected phase for a human decision: {other}"),
            }
        }
        let result = result.expect("seed 381's greedy self-play reaches a big gin");
        assert!(saw_move, "the UI receives a `big_gin` move to animate");
        assert!(
            core.log.iter().any(|l| l.contains("BIG GIN")),
            "the big gin is narrated in the log",
        );
        let summary = describe(result, core.game.rules());
        assert!(
            summary.contains("BIG gin"),
            "the round summary names the big gin: {summary}",
        );
        assert!(
            summary.contains(&format!("= {}", result.points(core.game.rules()))),
            "the summary spells out the round total: {summary}",
        );
    }
}
