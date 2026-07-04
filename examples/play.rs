//! Play gin rummy against a bot in the terminal.
//!
//! ```console
//! cargo run --release --example play
//! cargo run --release --example play -- --bot mc:64 --rules classic --seed 7
//! ```
//!
//! You are seated as Player One and see only legal information: your hand,
//! the pile, the stock count, and what the bot has revealed.  Cards parse
//! leniently: `S10`, `♠10`, `st`, and `♠T` all name the ten of spades, and a
//! lone rank or suit (`5`, `♠`) resolves when your hand holds just one such
//! card.  To move, type a card to discard it or `knock` (or `n`) to knock
//! the smallest deadwood.  A fully-melded hand declares big gin on its own.

use anyhow::{Context as _, Result, bail};
use gin_rummy::{Card, Hand, Phase, Player, Rank, RoundResult, Rules, Suit, best_melds, deadwood};
use gin_rummy_engine::{
    DrawAction, EngineError, HeuristicBot, Layoff, MonteCarloBot, Strategy, Table, TurnAction,
    UpcardAction, View,
};
use rand::rngs::StdRng;
use rand::{RngExt as _, SeedableRng};
use std::io::Write as _;

const HUMAN: Player = Player::One;

fn parse_args() -> Result<(String, Rules, Option<u64>)> {
    let mut bot = "mc".to_string();
    let mut rules = Rules::default();
    let mut seed = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--bot" => bot = value()?,
            "--seed" => seed = Some(value()?.parse()?),
            "--rules" => {
                rules = match value()?.as_str() {
                    "modern" => Rules::new(),
                    "classic" => Rules::classic(),
                    "palace" => Rules::palace(),
                    other => bail!("unknown rules preset {other:?}"),
                }
            }
            other => bail!("unknown flag {other:?} (--bot/--seed/--rules)"),
        }
    }
    Ok((bot, rules, seed))
}

fn make_bot(spec: &str, rng: &mut StdRng) -> Result<Box<dyn Strategy>> {
    let (kind, samples) = match spec.split_once(':') {
        Some((kind, samples)) => (kind, Some(samples.parse::<u32>()?)),
        None => (spec, None),
    };
    match kind {
        "greedy" => Ok(Box::new(HeuristicBot::new())),
        "mc" => Ok(Box::new(
            MonteCarloBot::new(StdRng::seed_from_u64(rng.random())).samples(samples.unwrap_or(64)),
        )),
        other => bail!("unknown bot {other:?} (greedy | mc[:samples])"),
    }
}

/// Read one trimmed lowercase line, or `None` on end of input
fn read_command(prompt: &str) -> Option<String> {
    print!("{prompt} ");
    std::io::stdout().flush().ok()?;
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        // EOF (Ctrl-D) leaves the cursor mid-prompt; close the line first.
        Ok(0) | Err(_) => {
            println!();
            None
        }
        Ok(_) => Some(line.trim().to_lowercase()),
    }
}

/// Resolve user text to a card.  A full name (`S10`, `♠T`) is taken as
/// written; a lone rank (`5`) or suit (`♠`, `s`) resolves only when the hand
/// holds exactly one matching card, so the common case needs no full name.
fn resolve_card(text: &str, hand: Hand) -> Option<Card> {
    if let Ok(card) = text.parse::<Card>() {
        return Some(card);
    }
    let matches: Vec<Card> = if let Ok(rank) = text.parse::<Rank>() {
        hand.iter().filter(|c| c.rank == rank).collect()
    } else if let Ok(suit) = text.parse::<Suit>() {
        hand.iter().filter(|c| c.suit == suit).collect()
    } else {
        Vec::new()
    };
    match matches.as_slice() {
        [card] => Some(*card),
        [] => {
            println!("Cannot read {text:?} as a card; try forms like S10 or ♠T.");
            None
        }
        many => {
            let names: Vec<String> = many.iter().map(Card::to_string).collect();
            println!("{text:?} matches several cards: {}.", names.join(" "));
            None
        }
    }
}

/// The largest-pip deadwood card, skipping the just-taken card: shedding it
/// leaves the smallest residual to knock on, so it is the card to auto-knock.
///
// ponytail: mirrors the crate-private greedy `best_shed`, inlined because
// examples see only the public API.
fn best_shed(hand: Hand, taken: Option<Card>) -> Card {
    hand.iter()
        .filter(|&c| Some(c) != taken)
        .min_by_key(|&c| (deadwood(hand - c.into()), u8::MAX - c.rank.deadwood()))
        .expect("a full hand always has a legal discard")
}

/// Wrap a card's name in ANSI reverse video wherever it appears in `text`,
/// so the just-drawn card is easy to pick out of the hand.  A card's name is
/// unique within a hand, so the plain substring replace never mis-hits.
fn emphasize(text: &str, card: Card) -> String {
    let name = card.to_string();
    text.replace(&name, &format!("\x1b[7m{name}\x1b[0m"))
}

/// A card set as a spaced list, friendlier than the dotted suit groups
/// for the short sets shown here
fn list(cards: gin_rummy::Hand) -> String {
    cards
        .iter()
        .map(|card| card.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A hand as a spaced list.  `Hand::iter` already runs by suit (clubs first,
/// ascending); the default reorders by rank across suits, which is how most
/// players scan a hand.
fn sorted(cards: gin_rummy::Hand, by_suit: bool) -> String {
    let mut cards: Vec<Card> = cards.iter().collect();
    if !by_suit {
        cards.sort_by_key(|card| (card.rank, card.suit));
    }
    cards
        .iter()
        .map(|card| card.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The player's own hand on one line: the best meld arrangement, then the
/// loose deadwood ordered by rank (the default) or by suit.  `drawn`, if set,
/// is the just-drawn card, highlighted so the human can track it.
fn print_hand(view: &View<'_>, by_suit: bool, drawn: Option<Card>) {
    let melds = view.best_melds();
    let arranged = melds
        .iter()
        .map(|m| m.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let loose = melds.deadwood_cards();
    // Mirror `Melds` Display: the `|` only separates two non-empty sides.
    let sep = if !arranged.is_empty() && !loose.is_empty() {
        " | "
    } else {
        ""
    };
    let line = format!(
        "Your hand: {arranged}{sep}{} ({} deadwood)",
        sorted(loose, by_suit),
        melds.deadwood(),
    );
    match drawn {
        Some(card) => println!("{}", emphasize(&line, card)),
        None => println!("{line}"),
    }
}

/// Show everything the human may see before a decision
fn show_position(view: &View<'_>, by_suit: bool, drawn: Option<Card>) {
    println!();
    print_hand(view, by_suit, drawn);
    match view.upcard() {
        Some(top) => println!(
            "Pile top: {top} (pile of {}), stock: {} cards",
            view.discard_pile().len(),
            view.stock_len(),
        ),
        None => println!("Pile empty, stock: {} cards", view.stock_len()),
    }
    if !view.opponent_known().is_empty() {
        println!("Bot is holding: {}", list(view.opponent_known()));
    }
}

/// Interactive human seat.  `by_suit` toggles the deadwood ordering between
/// by-rank (the default) and by-suit via the `sort` command.
struct HumanCli {
    by_suit: bool,
    /// The hand just before the current draw, so `drawn` can name the card
    /// the draw added.
    predraw: Option<Hand>,
}

impl HumanCli {
    /// Flip the hand ordering and reprint it; a display-only command.
    fn toggle_sort(&mut self, view: &View<'_>) {
        self.by_suit = !self.by_suit;
        print_hand(view, self.by_suit, self.drawn(view));
    }

    /// The card added since the last draw decision, if any — the one to
    /// highlight so the human can track what they just drew.  Empty (`None`)
    /// during the draw itself, when the snapshot equals the current hand.
    fn drawn(&self, view: &View<'_>) -> Option<Card> {
        self.predraw
            .and_then(|before| (view.hand() - before).iter().next())
    }
}

impl Strategy for HumanCli {
    fn offer_upcard(&mut self, view: &View<'_>) -> UpcardAction {
        self.predraw = Some(view.hand());
        show_position(view, self.by_suit, self.drawn(view));
        loop {
            match read_command("Take the upcard or pass? [take/pass]")
                .unwrap_or_else(|| "quit".into())
                .as_str()
            {
                "take" | "t" => return UpcardAction::Take,
                "pass" | "p" => return UpcardAction::Pass,
                "sort" | "view" => self.toggle_sort(view),
                "quit" => std::process::exit(0),
                _ => println!("Commands: take, pass, sort, quit."),
            }
        }
    }

    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction {
        self.predraw = Some(view.hand());
        show_position(view, self.by_suit, self.drawn(view));
        loop {
            match read_command("Draw from the stock or take the pile top? [draw/take]")
                .unwrap_or_else(|| "quit".into())
                .as_str()
            {
                "draw" | "d" => return DrawAction::Stock,
                "take" | "t" => return DrawAction::TakeDiscard,
                "sort" | "view" => self.toggle_sort(view),
                "quit" => std::process::exit(0),
                _ => println!("Commands: draw, take, sort, quit."),
            }
        }
    }

    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        show_position(view, self.by_suit, self.drawn(view));
        // Big gin — all eleven cards melded — is strictly dominant, so
        // declare it without asking, as the bots do.  Rulesets without a
        // big-gin bonus fall through to a normal gin knock.
        if view.deadwood() == 0 && view.rules().big_gin_bonus.is_some() {
            println!("You have BIG GIN!");
            return TurnAction::BigGin(view.best_melds());
        }
        if let Some(card) = view.taken_discard() {
            println!("(The just-taken {card} may not be shed this turn.)");
        }
        loop {
            match read_command("Your move [<card> to discard / k\x1b[7mn\x1b[0mock]:")
                .unwrap_or_else(|| "quit".into())
                .as_str()
            {
                // Knock is `n` (a bare `k` names your only king).  The shed is
                // forced, not chosen: it goes face down, so it never reaches
                // the opponent, leaving the knocker's own deadwood as the only
                // objective — and `best_shed` minimizes it.
                "knock" | "n" => {
                    let discard = best_shed(view.hand(), view.taken_discard());
                    return TurnAction::Knock {
                        discard,
                        melds: best_melds(view.hand() - discard.into()),
                    };
                }
                "sort" | "view" => self.toggle_sort(view),
                // Quit has no `q` shortcut: a bare `q` names your only queen.
                "quit" => std::process::exit(0),
                // A bare card is a discard; no `discard` command needed.
                text => {
                    if let Some(card) = resolve_card(text, view.hand()) {
                        return TurnAction::Discard(card);
                    }
                }
            }
        }
    }

    fn choose_layoff(&mut self, view: &View<'_>) -> Option<Layoff> {
        // ponytail: reuse the bot's greedy layoff; the human need not choose.
        let layoff = HeuristicBot::new().choose_layoff(view);
        if let Some(l) = &layoff {
            println!("You lay off {}.", l.card);
        }
        layoff
    }

    fn name(&self) -> &str {
        "you"
    }
}

/// Narrate what the bot just did from public information alone
fn narrate_bot(before_phase: Phase, before_top: Option<Card>, before_pile: usize, table: &Table) {
    let round = table.round();
    match before_phase {
        Phase::Upcard => {
            if round.phase() == Phase::Discard {
                println!(
                    "Bot takes the {}.",
                    before_top.expect("a take implies a pile top"),
                );
            } else {
                println!("Bot passes.");
            }
        }
        Phase::Draw => {
            if round.discard_pile().len() < before_pile {
                println!(
                    "Bot takes the {}.",
                    before_top.expect("a take implies a pile top"),
                );
            } else {
                println!("Bot draws from the stock.");
            }
        }
        Phase::Discard => {
            let top = round.discard_pile().last().copied();
            match round.result() {
                Some(RoundResult::BigGin { .. }) => println!("Bot declares BIG GIN!"),
                Some(RoundResult::Gin { .. }) => {
                    println!(
                        "Bot discards {} and goes gin!",
                        top.expect("gin sheds a discard"),
                    );
                }
                _ if round.knocker() == Some(Player::Two) => {
                    println!(
                        "Bot discards {} and knocks.",
                        top.expect("a knock sheds a discard"),
                    );
                    let spread = table
                        .view(HUMAN)
                        .spread()
                        .map(|meld| meld.to_string())
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!("Bot spreads: {spread}");
                }
                _ => println!("Bot discards {}.", top.expect("a turn ends with a discard")),
            }
        }
        Phase::Layoff => {
            let laid = round.laid_off();
            if round.phase() == Phase::Layoff {
                println!("Bot lays off (so far: {}).", list(laid));
            } else if laid.is_empty() {
                println!("Bot lays nothing off.");
            } else {
                println!("Bot lays off {}.", list(laid));
            }
        }
        Phase::Finished => {}
    }
}

fn describe(result: RoundResult) -> String {
    match result {
        RoundResult::Dead => "dead hand, nobody scores".into(),
        RoundResult::Knock { winner, margin } => format!("{} knock for {margin}", who(winner)),
        RoundResult::Undercut { winner, margin } => {
            format!("{} undercut by {margin}", who(winner))
        }
        RoundResult::Gin { winner, deadwood } => format!("{} gin (+{deadwood})", who(winner)),
        RoundResult::BigGin { winner, deadwood } => {
            format!("{} BIG gin (+{deadwood})", who(winner))
        }
        _ => format!("{result:?}"),
    }
}

fn who(player: Player) -> &'static str {
    if player == HUMAN { "You" } else { "Bot" }
}

fn main() -> Result<()> {
    let (spec, rules, seed) = parse_args()?;
    let mut rng = match seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => StdRng::from_rng(&mut rand::rng()),
    };
    let mut bot = make_bot(&spec, &mut rng)?;
    let mut human = HumanCli {
        by_suit: false,
        predraw: None,
    };
    let mut game = gin_rummy::Game::new(rules, HUMAN);

    println!("Gin rummy to {} points — you vs {spec}.", rules.game_target);

    for number in 1.. {
        if game.is_over() {
            break;
        }
        println!();
        println!(
            "=== Round {number} ({} deal) ===",
            if game.next_dealer() == HUMAN {
                "your"
            } else {
                "bot's"
            },
        );
        let mut table = Table::new(game.deal(&mut rng));

        while let Some(seat) = table.turn() {
            if seat == HUMAN {
                if let Err(EngineError::IllegalAction { source, .. }) = table.step(&mut human) {
                    println!("Illegal: {source}");
                }
            } else {
                let phase = table.round().phase();
                let top = table.round().discard_pile().last().copied();
                let pile = table.round().discard_pile().len();
                table
                    .step(&mut *bot)
                    .expect("the bot always chooses legal actions");
                narrate_bot(phase, top, pile, &table);
            }
        }

        let result = table.round().result().expect("a turnless round finished");
        game.record(result)?;
        println!();
        println!("Round {number}: {}", describe(result));
        println!(
            "Score — you {} : {} bot",
            game.score(HUMAN),
            game.score(HUMAN.opponent()),
        );
    }

    let settled = game.final_score().expect("the game just ended");
    println!();
    println!(
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
    );
    Ok(())
}
