#![cfg(feature = "rand")]

#[path = "../examples/support/strong/marjj.rs"]
mod marjj;
#[path = "../examples/support/strong/melds.rs"]
mod melds;

use gin_rummy::{Card, Hand, Player, Round, Rules};
use gin_rummy_engine::{HeuristicBot, Strategy, Table, TurnAction};
use marjj::MarjjV5Surrogate;
use rand::SeedableRng as _;

fn card(text: &str) -> Card {
    text.parse().expect("a valid fixture card")
}

fn hand(cards: &[&str]) -> Hand {
    cards
        .iter()
        .map(|text| card(text))
        .fold(Hand::EMPTY, |cards, card| cards | card.into())
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
fn enumerates_every_equal_deadwood_partition_in_canonical_order() {
    // Either meld C4-C5-C6 or the three fives.  Both leave ten deadwood.
    let cards = hand(&["4C", "5C", "6C", "5H", "5S"]);
    let partitions = melds::all_minimum_melds(cards);

    assert_eq!(partitions.len(), 2);
    assert!(
        partitions
            .iter()
            .all(|partition| partition.deadwood() == 10)
    );
    assert_eq!(partitions[0].melded(), hand(&["4C", "5C", "6C"]));
    assert_eq!(partitions[1].melded(), hand(&["5C", "5H", "5S"]));
}

#[test]
fn draw_gate_uses_layoff_estimate_then_canonical_order_after_strict_improvement() {
    let fillers = ["AD", "3D", "8D", "QD", "2H", "KH"];

    // Adding H5 creates equal run/set alternatives and strictly improves the
    // best post-discard deadwood.  With all neighboring cards still unseen,
    // v5 estimates fewer layoffs against the set and therefore accepts H5.
    let mut set_selected = hand(&["4C", "5C", "6C", "5S"]);
    set_selected |= hand(&fillers);
    let top = card("5H");
    let seen = set_selected | top.into();
    assert_eq!(set_selected.len(), 10);
    assert!(MarjjV5Surrogate::<rand::rngs::StdRng>::would_take(
        set_selected,
        top,
        seen,
        Hand::EMPTY,
    ));

    // The same state-sensitive choice rejects C4: it improves deadwood but
    // appears only in the run, while the lower-layoff set is selected.
    let mut set_rejects_top = hand(&["5C", "6C", "5H", "5S"]);
    set_rejects_top |= hand(&fillers);
    let top = card("4C");
    let seen = set_rejects_top | top.into();
    assert!(!MarjjV5Surrogate::<rand::rngs::StdRng>::would_take(
        set_rejects_top,
        top,
        seen,
        Hand::EMPTY,
    ));

    // Once both possible run-extension cards are known unavailable, the run
    // has the lower layoff estimate and the exact same C4 is accepted.
    let seen = seen | hand(&["3C", "7C"]);
    assert!(MarjjV5Surrogate::<rand::rngs::StdRng>::would_take(
        set_rejects_top,
        top,
        seen,
        Hand::EMPTY,
    ));
}

#[test]
fn discard_filter_short_circuits_on_the_first_canonical_gin_card() {
    let clubs = hand(&[
        "AC", "2C", "3C", "4C", "5C", "6C", "7C", "8C", "9C", "TC", "JC",
    ]);
    let candidates = MarjjV5Surrogate::<rand::rngs::StdRng>::discard_candidates(clubs, None);
    assert_eq!(candidates, vec![(card("AC"), 0)]);
}

#[test]
fn danger_score_preserves_rank_and_queen_boundary_bugs() {
    let seven = card("7C");
    let own_seven = Hand::from(seven);
    // The candidate itself is the sole unavailable same-rank card.  V5's
    // mistaken condition therefore adds a rank penalty even with no known
    // opponent card of that rank.
    assert_eq!(
        MarjjV5Surrogate::<rand::rngs::StdRng>::danger_score(
            seven,
            own_seven,
            own_seven,
            Hand::EMPTY,
        ),
        4.0
    );

    // At a queen, the out-of-range distance-two check marks the one-away
    // king unavailable.  That contributes -5, then the rank bug adds +4.
    let queen = card("QC");
    let own_queen = Hand::from(queen);
    assert_eq!(
        MarjjV5Surrogate::<rand::rngs::StdRng>::danger_score(
            queen,
            own_queen,
            own_queen,
            Hand::EMPTY,
        ),
        -1.0
    );

    // Pile takes remain in v5's opponent mask after being shed and therefore
    // overlap `seen`.  Opponent membership wins that classification: stale
    // 6C/8C still make discarding 7C look like a known sequence (+20), in
    // addition to the mistaken +4 rank penalty.
    let stale_opponent = hand(&["6C", "8C"]);
    assert_eq!(
        MarjjV5Surrogate::<rand::rngs::StdRng>::danger_score(
            seven,
            own_seven,
            own_seven | stale_opponent,
            stale_opponent,
        ),
        24.0
    );
}

#[test]
fn public_v5_diagnostic_fixture_matches_score_components() {
    // Scenario 2 embedded in public commit 5d1f00c.  The Java `main` passes
    // turn zero (and therefore an unusual negative decay exponent); compare
    // the invariant component values here at the real first-turn count.
    let cards = hand(&[
        "QC", "QD", "QS", "AS", "3H", "5D", "8H", "8S", "TS", "JS", "JH",
    ]);
    let evaluations = MarjjV5Surrogate::<rand::rngs::StdRng>::evaluate_discards(
        cards,
        None,
        cards,
        Hand::EMPTY,
        1,
    );
    let expected = [
        ("QC", 45, 208, -15.0),
        ("8H", 47, 173, -4.0),
        ("JH", 45, 171, -4.0),
        ("8S", 47, 181, -7.0),
        ("TS", 45, 179, -6.0),
        ("JS", 45, 198, -14.0),
        ("5D", 50, 160, 4.0),
        ("QD", 45, 208, -15.0),
    ];

    assert_eq!(evaluations.len(), expected.len());
    for (evaluation, (name, post_deadwood, future_sum, danger)) in evaluations.iter().zip(expected)
    {
        assert_eq!(evaluation.card, card(name));
        assert_eq!(evaluation.post_deadwood, post_deadwood);
        assert_eq!(evaluation.future_mean, f64::from(future_sum) / 7.0);
        assert_eq!(evaluation.danger, danger);
        assert_eq!(
            evaluation.total,
            f64::from(post_deadwood) + 18.0 * (f64::from(future_sum) / 7.0) + danger
        );
    }
}

#[test]
fn knock_window_is_three_turns_then_gin_only() {
    type Bot = MarjjV5Surrogate<rand::rngs::StdRng>;
    assert!(Bot::should_knock(1, 10, 10));
    assert!(Bot::should_knock(3, 7, 10));
    assert!(!Bot::should_knock(4, 1, 10));
    assert!(Bot::should_knock(20, 0, 10));
    assert!(!Bot::should_knock(1, 7, 5));
}

#[test]
fn injected_seed_constructs_a_replayable_named_strategy() {
    let bot = MarjjV5Surrogate::new(rand::rngs::StdRng::seed_from_u64(2021));
    assert_eq!(bot.name(), "marjj-v5-surrogate");
}

#[test]
fn seeded_whole_round_is_legal_and_replayable() {
    let mut deal_rng = rand::rngs::StdRng::seed_from_u64(14);
    let round = Round::deal(Rules::default(), Player::One, &mut deal_rng);

    let play = |round| {
        let mut marjj = MarjjV5Surrogate::new(rand::rngs::StdRng::seed_from_u64(2021));
        let mut opponent = HeuristicBot::new();
        Table::new(round)
            .play([&mut marjj, &mut opponent])
            .expect("the surrogate emits legal host actions")
    };

    assert_eq!(play(round.clone()), play(round));
}

#[test]
fn exact_best_ties_use_only_the_injected_rng() {
    // A fixed dealt eleven-card fixture has an exact best-score tie between
    // KC and QH.  Equal seeds replay, while a small seed sweep reaches both
    // uniformly eligible choices and never escapes the tied minimum.
    let cards = hand(&[
        "2C", "KC", "2H", "3H", "7H", "QH", "AS", "3S", "4S", "7S", "8S",
    ]);
    let choose = |seed| {
        let mut bot = MarjjV5Surrogate::new(rand::rngs::StdRng::seed_from_u64(seed));
        bot.choose_discard_for_state(cards, None, cards, Hand::EMPTY, 1)
            .0
    };

    assert_eq!(choose(2021), choose(2021));
    let choices: Hand = (0..32).map(choose).collect();
    assert_eq!(choices, hand(&["KC", "QH"]));
}

#[test]
fn knock_uses_the_optimal_spread_with_the_lower_legacy_layoff_estimate() {
    let initial = hand(&["9C", "TC", "QC", "KC", "9H", "KH", "2S", "8S", "9S", "9D"]);
    let mut table = Table::new(round_with_upcard(initial, card("JC")));
    let mut bot = MarjjV5Surrogate::new(rand::rngs::StdRng::seed_from_u64(2021));

    table
        .step(&mut bot)
        .expect("MARJJ takes the improving opening jack");
    let TurnAction::Knock { discard, melds } = bot.play_turn(&table.view(Player::One)) else {
        panic!("the first-turn ten-deadwood position is a voluntary knock");
    };

    assert_eq!(discard, card("KH"));
    let remaining = table.view(Player::One).hand() - discard.into();
    let partitions = melds::all_minimum_melds(remaining);
    assert_eq!(partitions.len(), 2);
    assert!(partitions.iter().all(|spread| spread.deadwood() == 10));

    // The selected split has no available layoff points: C9 is already in
    // the four-nine set, blocking the low end of the CT-CK run.  The other
    // minimum-deadwood split exposes C8 below its C9-CK run, which v5's
    // legacy boundary arithmetic estimates at nine points.
    let selected: Vec<Hand> = melds.iter().map(|meld| meld.cards()).collect();
    assert_eq!(
        selected,
        vec![
            hand(&["TC", "JC", "QC", "KC"]),
            hand(&["9C", "9H", "9S", "9D"]),
        ]
    );
}
