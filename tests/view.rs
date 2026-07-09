//! Information hygiene: what a `View` reveals and what it never can

use gin_rummy::{Hand, Phase, Player, Round, Rules};
use gin_rummy_engine::{
    DrawAction, HeuristicBot, Layoff, Strategy, Table, TurnAction, UpcardAction, View,
};

/// A scripted seat: fixed upcard and draw choices, sheds the first legal
/// card, never lays off
struct Script {
    upcard: UpcardAction,
    draw: DrawAction,
}

impl Strategy for Script {
    fn offer_upcard(&mut self, _: &View<'_>) -> UpcardAction {
        self.upcard
    }
    fn choose_draw(&mut self, _: &View<'_>) -> DrawAction {
        self.draw
    }
    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        let card = view
            .hand()
            .iter()
            .find(|&card| Some(card) != view.taken_discard())
            .expect("an 11-card hand has a legal shed");
        TurnAction::Discard(card)
    }
    fn choose_layoff(&mut self, _: &View<'_>) -> Option<Layoff> {
        None
    }
}

fn fixed_deal(dealer: Player) -> Round {
    let deck: Vec<_> = Hand::ALL.iter().collect();
    let hands = [
        deck.iter().step_by(2).take(10).copied().collect::<Hand>(),
        deck.iter().skip(1).step_by(2).take(10).copied().collect(),
    ];
    Round::from_deal(
        Rules::default(),
        dealer,
        hands,
        deck[20],
        deck[21..].to_vec(),
    )
    .expect("a partitioned deck")
}

/// Everything a view must satisfy at any point of any round
fn assert_hygiene(table: &Table) {
    // The game score is public: the seats see mirrored totals.
    let [a, b] = table.view(Player::One).game_scores();
    assert_eq!(table.view(Player::Two).game_scores(), [b, a]);
    for seat in Player::ALL {
        let view = table.view(seat);
        let opponent = seat.opponent();
        let round = table.round();

        assert_eq!(view.hand(), round.hand(seat));
        assert_eq!(view.stock_len(), round.stock().len());
        assert_eq!(view.opponent_hand_len(), round.hand(opponent).len());

        // Knowledge is sound: known cards are truly in the opponent's
        // hand, and nothing the opponent shed hides in the stock.
        let known = view.opponent_known();
        assert_eq!(known & round.hand(opponent), known);
        let stock: Hand = round.stock().iter().copied().collect();
        assert!((view.opponent_shed() & stock).is_empty());

        // The unseen set is exactly the cards this seat cannot place.
        let unseen = view.unseen();
        assert!((unseen & view.hand()).is_empty());
        assert!((unseen & known).is_empty());
        let pile: Hand = view.discard_pile().iter().copied().collect();
        assert!((unseen & pile).is_empty());
        if round.knocker().is_none() {
            assert_eq!(
                unseen.len(),
                view.stock_len() + view.opponent_hand_len() - known.len(),
            );
        }
    }
}

#[test]
fn taking_and_shedding_updates_the_opponents_knowledge() {
    let mut table = Table::new(fixed_deal(Player::One));
    let upcard = table.view(Player::Two).upcard().expect("a fresh upcard");
    let mut taker = Script {
        upcard: UpcardAction::Take,
        draw: DrawAction::TakeDiscard,
    };

    // The non-dealer takes the upcard: it is known to the opponent and
    // marked unsheddable for the taker.
    assert_hygiene(&table);
    table.step(&mut taker).expect("taking the upcard is legal");
    assert_hygiene(&table);
    assert_eq!(table.view(Player::One).opponent_known(), upcard.into());
    assert_eq!(table.view(Player::Two).taken_discard(), Some(upcard));
    assert_eq!(table.view(Player::Two).opponent_known(), Hand::EMPTY);

    // The taker sheds a different card: the upcard stays known, the shed
    // card is recorded, and the turn state clears.
    table.step(&mut taker).expect("shedding is legal");
    assert_hygiene(&table);
    let shed = *table.round().discard_pile().last().expect("a fresh shed");
    assert_ne!(shed, upcard);
    assert_eq!(table.view(Player::One).opponent_known(), upcard.into());
    assert_eq!(table.view(Player::One).opponent_shed(), shed.into());
    assert_eq!(table.view(Player::Two).taken_discard(), None);
}

#[test]
fn double_pass_forces_the_stock_draw() {
    let mut table = Table::new(fixed_deal(Player::One));
    let upcard = table.view(Player::One).upcard().expect("a fresh upcard");
    let mut passer = Script {
        upcard: UpcardAction::Pass,
        draw: DrawAction::TakeDiscard,
    };

    table.step(&mut passer).expect("passing is legal");
    assert_hygiene(&table);
    assert_eq!(table.view(Player::One).opponent_passed(), upcard.into());

    table.step(&mut passer).expect("passing is legal");
    assert_hygiene(&table);
    assert_eq!(table.view(Player::Two).opponent_passed(), upcard.into());

    // The non-dealer is on the forced stock draw: the pile may not be
    // taken, and the driver draws without consulting the strategy (the
    // script would illegally take the discard otherwise).
    let view = table.view(Player::Two);
    assert_eq!(view.phase(), Phase::Draw);
    assert!(!view.can_take_discard());
    table
        .step(&mut passer)
        .expect("the driver draws from the stock");
    assert_hygiene(&table);
    assert_eq!(table.round().phase(), Phase::Discard);
    assert_eq!(table.round().hand(Player::Two).len(), 11);

    // Normal draws may take the pile again afterwards.
    table.step(&mut passer).expect("shedding is legal");
    assert!(table.view(Player::One).can_take_discard());
}

#[test]
fn game_scores_are_seat_relative() {
    // A standalone round has no scoreboard: both seats see a level game.
    let table = Table::new(fixed_deal(Player::One));
    assert_eq!(table.view(Player::One).game_scores(), [0, 0]);
    assert_eq!(table.view(Player::Two).game_scores(), [0, 0]);

    // With running totals attached, each seat sees its own totals first.
    let table = Table::new(fixed_deal(Player::One)).scores([40, 25]);
    assert_eq!(table.view(Player::One).game_scores(), [40, 25]);
    assert_eq!(table.view(Player::Two).game_scores(), [25, 40]);
}

#[test]
fn full_rounds_stay_hygienic() {
    for dealer in Player::ALL {
        let mut table = Table::new(fixed_deal(dealer));
        let mut bots = [HeuristicBot::new(), HeuristicBot::new()];
        while let Some(seat) = table.turn() {
            assert_hygiene(&table);
            table
                .step(&mut bots[seat as usize])
                .expect("heuristic bots play legally");
        }
        assert_hygiene(&table);
        assert!(table.round().result().is_some());
    }
}
