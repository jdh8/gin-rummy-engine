//! Deterministic conformance fixtures for the benchmark-only Gold adapter.

#![cfg(feature = "rand")]

#[path = "../examples/support/strong/mod.rs"]
mod strong;

use gin_rummy::{
    Card, Hand, Meld, Player, Rank, Round, RoundResult, Rules, Suit, best_melds, deadwood,
};
use gin_rummy_engine::{
    DrawAction, HeuristicBot, Layoff, Strategy, Table, TurnAction, UpcardAction,
};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use strong::GoldPaperBot;

fn card(text: &str) -> Card {
    text.parse().expect("a valid card")
}

fn hand(text: &str) -> Hand {
    text.parse().expect("a valid hand")
}

fn round_with_upcard(one: Hand, upcard: Card) -> Round {
    assert_eq!(one.len(), 10);
    let two = Hand::ALL
        .iter()
        .filter(|&candidate| !one.contains(candidate) && candidate != upcard)
        .take(10)
        .collect::<Hand>();
    let stock = Hand::ALL
        .iter()
        .filter(|&candidate| {
            !one.contains(candidate) && !two.contains(candidate) && candidate != upcard
        })
        .collect();
    Round::from_deal(Rules::default(), Player::Two, [one, two], upcard, stock)
        .expect("a partitioned deck")
}

#[test]
fn visible_discard_requires_strict_improvement() {
    let improving = hand("A2.456.789.5K");
    let table = Table::new(round_with_upcard(improving, card("C3")));
    let mut bot = GoldPaperBot::new();
    assert_eq!(
        bot.offer_upcard(&table.view(Player::One)),
        UpcardAction::Take
    );
    assert_eq!(
        bot.choose_draw(&table.view(Player::One)),
        DrawAction::TakeDiscard
    );

    let useless = hand("A2.456.789.5K");
    let table = Table::new(round_with_upcard(useless, card("DT")));
    let with = useless | card("DT").into();
    let best_take = with
        .iter()
        .filter(|&discard| discard != card("DT"))
        .map(|discard| deadwood(with - discard.into()))
        .min()
        .expect("taking into a ten-card hand leaves legal sheds");
    assert_eq!(best_take, deadwood(useless));
    assert_eq!(
        bot.offer_upcard(&table.view(Player::One)),
        UpcardAction::Pass
    );
    assert_eq!(bot.choose_draw(&table.view(Player::One)), DrawAction::Stock);
}

#[test]
fn ordinary_ties_dump_pips_then_follow_rlcard_order() {
    // With equal residual deadwood, higher pips precede RLCard id.
    assert!(GoldPaperBot::ordinary_key(card("CK"), 5) < GoldPaperBot::ordinary_key(card("S4"), 5));

    // Same pips and residual deadwood: spades has the lower RLCard id even
    // though the engine iterates diamonds first in this position.
    let suit_tie = hand("4.A24JQ.A56.7Q");
    assert_eq!(
        GoldPaperBot::ordinary_shed(suit_tie, None),
        (card("SQ"), 50)
    );
}

#[test]
fn knock_tie_omits_the_ordinary_pip_preference() {
    // At equal deadwood, the ordinary key prefers the king's pips, while
    // the knock key goes directly to the spade four's lower RLCard id.
    assert!(GoldPaperBot::ordinary_key(card("CK"), 5) < GoldPaperBot::ordinary_key(card("S4"), 5));
    assert!(GoldPaperBot::knock_key(card("S4"), 5) < GoldPaperBot::knock_key(card("CK"), 5));

    // With equal ten-point cards, both keys settle on the lower id.
    let position = hand("A23.456.789.JK");
    let ordinary = GoldPaperBot::ordinary_shed(position, None);
    let knock = GoldPaperBot::knock_shed(position, None);
    assert_eq!(ordinary, (card("SJ"), 10));
    assert_eq!(knock, (card("SJ"), 10));
}

#[test]
fn just_taken_card_is_never_shed() {
    let position = hand("A23.456.789.JK");
    assert_eq!(
        GoldPaperBot::ordinary_shed(position, Some(card("SK"))).0,
        card("SJ")
    );
    assert_ne!(
        GoldPaperBot::knock_shed(position, Some(card("SJ"))).0,
        card("SJ")
    );
}

#[test]
fn knocks_at_ten_but_not_eleven_deadwood() {
    let ten = hand("A23.456.789.JK");
    assert!(matches!(
        GoldPaperBot::turn_action(ten, None, 10),
        TurnAction::Knock { .. }
    ));
    assert!(matches!(
        GoldPaperBot::turn_action(ten, None, 9),
        TurnAction::Discard(_)
    ));

    // Find a compact deterministic 11-card fixture with minimum residual
    // deadwood exactly eleven; the chosen action must remain a discard.
    let eleven = hand("6T.AT.A567Q.3T");
    let (_, rest) = GoldPaperBot::ordinary_shed(eleven, None);
    assert_eq!(rest, 11);
    assert!(matches!(
        GoldPaperBot::turn_action(eleven, None, 10),
        TurnAction::Discard(_)
    ));
}

#[test]
fn fully_melded_eleven_cards_discard_to_gin() {
    let position = hand("A23456789TJ...");
    assert_eq!(position.len(), 11);
    assert_eq!(deadwood(position), 0);
    let action = GoldPaperBot::turn_action(position, None, 10);
    let TurnAction::Knock { discard, melds } = action else {
        panic!("Gold adapts RLCard gin to a discard-and-knock declaration");
    };
    assert_eq!(melds, best_melds(position - discard.into()));
    assert_eq!(melds.deadwood(), 0);
}

#[test]
fn gin_discard_retains_the_ordinary_tie_break() {
    // Multiple sheds leave gin.  The ordinary policy chooses the ten; the
    // knock-only id tie-break would choose the six, but upstream handles gin
    // before its non-gin knock selection.
    let position = hand("456.TJQ..6789T");
    assert_eq!(GoldPaperBot::ordinary_shed(position, None), (card("ST"), 0));
    assert_eq!(GoldPaperBot::knock_shed(position, None), (card("S6"), 0));
    assert!(matches!(
        GoldPaperBot::turn_action(position, None, 10),
        TurnAction::Knock { discard, .. } if discard == card("ST")
    ));
}

#[test]
fn spread_is_the_local_canonical_best_arrangement() {
    let position = hand("A23.456.789.JK");
    let action = GoldPaperBot::turn_action(position, None, 10);
    let TurnAction::Knock { discard, melds } = action else {
        panic!("ten deadwood is a legal knock");
    };
    assert_eq!(melds, best_melds(position - discard.into()));
}

#[test]
fn layoff_uses_the_shared_local_greedy_policy() {
    let spread = [
        Meld::run(Suit::Clubs, Rank::new(5), Rank::new(7)),
        Meld::set(Rank::new(9), Some(Suit::Spades)),
    ];
    // The spade nine stays inside the defender's own run; only the loose
    // club eight is eligible to extend the knocker's spread.
    assert_eq!(
        strong::greedy_layoff(hand("8...9TJQ"), spread.into_iter()),
        Some(Layoff {
            card: card("C8"),
            meld: 0,
        })
    );
}

#[test]
fn score_does_not_change_a_decision() {
    let position = hand("A2.456.789.5K");
    let round = round_with_upcard(position, card("C3"));
    let level = Table::new(round.clone());
    let trailing = Table::new(round).scores([0, 99]);
    let mut bot = GoldPaperBot::new();
    assert_eq!(
        bot.offer_upcard(&level.view(Player::One)),
        bot.offer_upcard(&trailing.view(Player::One))
    );
}

#[test]
fn seeded_rounds_are_legal_and_replay_exactly() {
    fn play(seed: u64) -> RoundResult {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut table = Table::deal(Rules::default(), Player::One, &mut rng);
        table
            .play([&mut GoldPaperBot::new(), &mut HeuristicBot::new()])
            .expect("both benchmark strategies choose legal actions")
    }

    for seed in 0..16 {
        assert_eq!(play(seed), play(seed));
    }
}

#[test]
fn strong_bot_factory_keeps_adapter_out_of_the_library_api() {
    let bot = strong::make_bot("gold-paper", 7)
        .expect("Gold construction is infallible")
        .expect("the strong-bot factory recognizes Gold");
    assert_eq!(bot.name(), "gold-paper");
    assert_eq!(strong::BOT_SPECS, &["gold-paper", "marjj-v5-surrogate"]);
    assert!(
        strong::make_bot("ordinary-arena-bot", 7)
            .expect("unknown specifications are not errors")
            .is_none()
    );
}
