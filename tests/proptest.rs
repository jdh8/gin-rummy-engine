//! Property tests: driven rounds terminate, partition the deck, and stay
//! hygienic under every ruleset

use gin_rummy::{Hand, OklahomaAce, Player, Round, Rules};
use gin_rummy_engine::{HeuristicBot, HeuristicConfig, Table};
use proptest::prelude::*;

fn full_deck() -> Vec<gin_rummy::Card> {
    Hand::ALL.iter().collect()
}

fn deal(deck: &[gin_rummy::Card], rules: Rules, dealer: Player) -> Round {
    let hands = [
        deck[..10].iter().copied().collect::<Hand>(),
        deck[10..20].iter().copied().collect(),
    ];
    Round::from_deal(rules, dealer, hands, deck[20], deck[21..].to_vec())
        .expect("a permutation of the deck deals cleanly")
}

/// The four zones every card must be in exactly once
fn assert_partition(table: &Table) {
    let round = table.round();
    let zones = [
        round.hand(Player::One),
        round.hand(Player::Two),
        round.stock().iter().copied().collect(),
        round.discard_pile().iter().copied().collect(),
        round.laid_off(),
    ];
    let mut seen = Hand::EMPTY;
    let mut total = 0;
    for zone in zones {
        assert!((seen & zone).is_empty(), "zones overlap");
        seen |= zone;
        total += zone.len();
    }
    // Laid-off cards also sit inside the knocker's hand-derived spread,
    // never anywhere else; the five zones tile the deck.
    assert_eq!(seen, Hand::ALL);
    assert_eq!(total, 52);
}

fn assert_unseen_identity(table: &Table) {
    // Once a knock reveals the spread, part of the opponent's hand is
    // public and the count identity no longer holds by design.
    if table.round().knocker().is_some() {
        return;
    }
    for seat in Player::ALL {
        let view = table.view(seat);
        assert_eq!(
            view.unseen().len(),
            view.stock_len() + view.opponent_hand_len() - view.opponent_known().len(),
        );
    }
}

proptest! {
    #[test]
    fn greedy_rounds_terminate_partitioned(
        deck in Just(full_deck()).prop_shuffle(),
        preset in 0..5usize,
        seat in 0..2usize,
        threshold in 0..=10u8,
        weight in 0..3u8,
    ) {
        let mut oklahoma_one = Rules::new();
        oklahoma_one.oklahoma = Some(OklahomaAce::One);
        let mut oklahoma_gin = Rules::new();
        oklahoma_gin.oklahoma = Some(OklahomaAce::GinOnly);
        let rules = [
            Rules::new(),
            Rules::classic(),
            Rules::palace(),
            oklahoma_one,
            oklahoma_gin,
        ][preset];
        let mut config = HeuristicConfig::default();
        config.knock_threshold = threshold;
        config.safety_weight = weight;
        let mut bots = [
            HeuristicBot::with_config(config),
            HeuristicBot::new(),
        ];

        let mut table = Table::new(deal(&deck, rules, Player::ALL[seat]));
        for _ in 0..200 {
            assert_partition(&table);
            assert_unseen_identity(&table);
            let Some(acting) = table.turn() else { break };
            table.step(&mut bots[acting as usize]).expect("legal play");
        }
        prop_assert!(table.round().result().is_some(), "the round must finish within 200 steps");
    }
}
