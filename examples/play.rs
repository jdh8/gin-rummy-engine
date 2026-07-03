//! Play gin rummy against a bot in the terminal.
//!
//! ```console
//! cargo run --release --example play
//! cargo run --release --example play -- --bot mc:64 --rules classic --seed 7
//! ```
//!
//! You are seated as Player One and see only legal information: your hand,
//! the pile, the stock count, and what the bot has revealed.  Cards parse
//! leniently: `S10`, `♠10`, `st`, and `♠T` all name the ten of spades.

use anyhow::{Context as _, Result, bail};
use gin_rummy::{Card, Phase, Player, RoundResult, Rules, best_melds};
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
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_lowercase()),
    }
}

fn parse_card(text: &str) -> Option<Card> {
    match text.parse() {
        Ok(card) => Some(card),
        Err(_) => {
            println!("Cannot read {text:?} as a card; try forms like S10 or ♠T.");
            None
        }
    }
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

/// Show everything the human may see before a decision
fn show_position(view: &View<'_>) {
    let melds = view.best_melds();
    println!();
    println!("Your hand: {melds} ({} deadwood)", melds.deadwood(),);
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

struct HumanCli;

impl Strategy for HumanCli {
    fn offer_upcard(&mut self, view: &View<'_>) -> UpcardAction {
        show_position(view);
        loop {
            match read_command("Take the upcard or pass? [take/pass]")
                .unwrap_or_else(|| "quit".into())
                .as_str()
            {
                "take" | "t" => return UpcardAction::Take,
                "pass" | "p" => return UpcardAction::Pass,
                "quit" | "q" => std::process::exit(0),
                _ => println!("Commands: take, pass, quit."),
            }
        }
    }

    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction {
        show_position(view);
        loop {
            match read_command("Draw from the stock or take the pile top? [draw/take]")
                .unwrap_or_else(|| "quit".into())
                .as_str()
            {
                "draw" | "d" => return DrawAction::Stock,
                "take" | "t" => return DrawAction::TakeDiscard,
                "quit" | "q" => std::process::exit(0),
                _ => println!("Commands: draw, take, quit."),
            }
        }
    }

    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        show_position(view);
        if let Some(card) = view.taken_discard() {
            println!("(The just-taken {card} may not be shed this turn.)");
        }
        loop {
            let line = read_command("Your move [discard <card> / knock <card> / gin]:")
                .unwrap_or_else(|| "quit".into());
            let (command, argument) = match line.split_once(' ') {
                Some((command, argument)) => (command, argument.trim()),
                None => (line.as_str(), ""),
            };
            match command {
                "discard" | "d" => {
                    if let Some(card) = parse_card(argument) {
                        return TurnAction::Discard(card);
                    }
                }
                "knock" | "k" => {
                    if let Some(card) = parse_card(argument) {
                        return TurnAction::Knock {
                            discard: card,
                            melds: best_melds(view.hand() - card.into()),
                        };
                    }
                }
                "gin" | "g" => return TurnAction::BigGin(view.best_melds()),
                "quit" | "q" => std::process::exit(0),
                _ => println!("Commands: discard <card>, knock <card>, gin, quit."),
            }
        }
    }

    fn choose_layoff(&mut self, view: &View<'_>) -> Option<Layoff> {
        println!();
        println!("The bot knocked and spread:");
        for (index, meld) in view.spread().enumerate() {
            println!("  [{index}] {}", meld.cards());
        }
        let melds = view.best_melds();
        println!("Your hand: {melds} ({} deadwood)", melds.deadwood());
        loop {
            let line = read_command("Lay off? [lay <card> <index> / done]:")
                .unwrap_or_else(|| "quit".into());
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                ["done"] | ["n"] => return None,
                ["lay" | "l", card, index] => {
                    let Some(card) = parse_card(card) else {
                        continue;
                    };
                    let Ok(meld) = index.parse() else {
                        println!("Cannot read {index:?} as a meld index.");
                        continue;
                    };
                    return Some(Layoff { card, meld });
                }
                ["quit"] | ["q"] => std::process::exit(0),
                _ => println!("Commands: lay <card> <meld index>, done, quit."),
            }
        }
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
    let mut human = HumanCli;
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
