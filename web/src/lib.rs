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
    DrawAction, HeuristicBot, Layoff, MonteCarloBot, Strategy, Table, TurnAction, UpcardAction,
    View,
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
    settled: Option<gin_rummy::FinalScore>,
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
        let table = Table::new(game.deal(&mut rng));
        let mut core = Self {
            game,
            table,
            bot,
            pending: Pending::default(),
            rng,
            round_no: 1,
            log: vec![round_header(1, dealer)],
            last_result: None,
            settled: None,
        };
        core.advance();
        core
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

    /// Stage a human action, step once, then advance to the next decision.
    fn human(&mut self, action: HumanAction) {
        self.pending.action = Some(action);
        if let Err(e) = self.table.step(&mut self.pending) {
            // A well-behaved UI only offers legal moves; surface anything else.
            self.log.push(format!("Illegal: {e}"));
        }
        self.advance();
    }

    /// Run bot turns (and the human's forced/auto steps) until the human has a
    /// real decision to make or the game is over.
    fn advance(&mut self) {
        loop {
            let Some(seat) = self.table.turn() else {
                if self.settled.is_some() {
                    break;
                }
                self.finish_round();
                if self.settled.is_some() {
                    break;
                }
                continue; // a fresh round was dealt; run its leading actions
            };

            if seat == HUMAN {
                let step = {
                    let view = self.table.view(HUMAN);
                    match view.phase() {
                        // The forced stock draw after both pass consults no one.
                        Phase::Draw if !view.can_take_discard() => HumanStep::AutoStep,
                        Phase::Layoff => HumanStep::AutoStep,
                        Phase::Discard
                            if view.deadwood() == 0 && view.rules().big_gin_bonus.is_some() =>
                        {
                            HumanStep::BigGin(view.best_melds())
                        }
                        Phase::Upcard | Phase::Draw | Phase::Discard => HumanStep::WaitInput,
                        Phase::Finished => HumanStep::AutoStep,
                    }
                };
                match step {
                    HumanStep::WaitInput => break,
                    HumanStep::AutoStep => {
                        let _ = self.table.step(&mut self.pending);
                    }
                    HumanStep::BigGin(melds) => {
                        self.log.push("You declare BIG GIN!".into());
                        self.pending.action = Some(HumanAction::Turn(TurnAction::BigGin(melds)));
                        let _ = self.table.step(&mut self.pending);
                    }
                }
            } else {
                let phase = self.table.round().phase();
                let top = self.table.round().discard_pile().last().copied();
                let pile = self.table.round().discard_pile().len();
                self.table
                    .step(&mut *self.bot)
                    .expect("the bot always chooses legal actions");
                self.narrate_bot(phase, top, pile);
            }
        }
    }

    /// Record the finished round, then deal the next one or settle the game.
    fn finish_round(&mut self) {
        let result = self
            .table
            .round()
            .result()
            .expect("a turnless round finished");
        self.last_result = Some(result);
        self.game
            .record(result)
            .expect("a result from the round it was dealt for records cleanly");
        self.log
            .push(format!("Round {}: {}.", self.round_no, describe(result)));
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
            self.round_no += 1;
            let dealer = self.game.next_dealer();
            self.table = Table::new(self.game.deal(&mut self.rng));
            self.log.push(round_header(self.round_no, dealer));
        }
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
        Snapshot {
            round_no: self.round_no,
            phase: phase_name(view.phase()),
            your_turn: self.table.turn() == Some(HUMAN) && !over,
            melds: arranged
                .iter()
                .map(|meld| cards_by_suit(meld.cards(), taken))
                .collect(),
            loose: cards_by_rank(arranged.deadwood_cards(), taken),
            deadwood: view.deadwood(),
            upcard: view.upcard().map(|c| card_json(c, None)),
            pile_len: view.discard_pile().len(),
            stock_len: view.stock_len(),
            opponent_known: cards_by_rank(view.opponent_known(), None),
            taken_discard: taken.map(|c| c.to_string()),
            you_score: self.game.score(HUMAN),
            bot_score: self.game.score(BOT),
            game_over: over,
            winner: self
                .settled
                .as_ref()
                .map(|s| if s.winner == HUMAN { "you" } else { "bot" }),
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

/// Everything one seat may legally see, plus the running transcript.
#[derive(Serialize)]
struct Snapshot {
    round_no: u32,
    phase: &'static str,
    your_turn: bool,
    /// The best meld arrangement, one inner list per meld.
    melds: Vec<Vec<CardJson>>,
    /// Loose deadwood, ordered by rank.
    loose: Vec<CardJson>,
    deadwood: u8,
    upcard: Option<CardJson>,
    pile_len: usize,
    stock_len: usize,
    opponent_known: Vec<CardJson>,
    /// `code` of the card that may not be shed this turn, if any.
    taken_discard: Option<String>,
    you_score: u16,
    bot_score: u16,
    game_over: bool,
    winner: Option<&'static str>,
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
    /// Start a game.  `bot` is `greedy` or `mc[:samples]`, `rules` is
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

fn describe(result: RoundResult) -> String {
    match result {
        RoundResult::Dead => "dead hand, nobody scores".into(),
        RoundResult::Knock { winner, margin } => format!("{} knock for {margin}", who(winner)),
        RoundResult::Undercut { winner, margin } => format!("{} undercut by {margin}", who(winner)),
        RoundResult::Gin { winner, deadwood } => format!("{} gin (+{deadwood})", who(winner)),
        RoundResult::BigGin { winner, deadwood } => {
            format!("{} BIG gin (+{deadwood})", who(winner))
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

    /// Drive a whole game to completion through the public decision methods,
    /// exercising the `Pending`-strategy loop, the auto steps, and the round
    /// hand-off without any browser.
    #[test]
    fn a_scripted_game_reaches_a_settled_score() {
        let mut core = Core::new("greedy", Rules::new(), 42);
        let mut guard = 0;
        while core.settled.is_none() {
            guard += 1;
            assert!(guard < 100_000, "the game must terminate");
            let snap = core.snapshot();
            assert!(snap.your_turn, "advance leaves the human to act or settles");
            match snap.phase {
                "upcard" => core.pass_upcard(),
                "draw" => core.draw_stock(),
                "discard" => {
                    let (card, can_knock) = {
                        let view = core.table.view(HUMAN);
                        (
                            best_shed(view.hand(), view.taken_discard()),
                            view.deadwood() <= view.knock_limit(),
                        )
                    };
                    // Knock when eligible — exercises knock() and the bot's
                    // defending layoff; otherwise shed the best card.
                    if can_knock {
                        core.knock();
                    } else {
                        core.discard(card);
                    }
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
}
