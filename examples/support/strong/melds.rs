//! Exhaustive, deterministic minimum-deadwood meld enumeration.
//!
//! The library's public solver returns one optimal spread.  MARJJ v5 uses
//! every optimal spread when it constructs discard candidates, so its host
//! adaptation needs the full set.  Ordering follows the EAAI framework's
//! card ids (clubs, hearts, spades, diamonds; then ascending rank), with
//! meld and partition ties settled by their corresponding bitsets.

use gin_rummy::{Card, Hand, Meld, Melds, Rank, Suit, pip_sum};

/// EAAI's suit cycle, which differs from this crate's display iteration.
pub(super) const EAAI_SUITS: [Suit; 4] = [Suit::Clubs, Suit::Hearts, Suit::Spades, Suit::Diamonds];

/// The EAAI framework card id used by MARJJ: C, H, S, D, then rank.
#[must_use]
pub(super) const fn eaai_id(card: Card) -> u8 {
    let suit = match card.suit {
        Suit::Clubs => 0,
        Suit::Hearts => 1,
        Suit::Spades => 2,
        Suit::Diamonds => 3,
    };
    suit * 13 + card.rank.get() - 1
}

/// The card represented by an EAAI framework id.
#[must_use]
pub(super) const fn card_from_eaai_id(id: u8) -> Card {
    Card {
        suit: EAAI_SUITS[(id / 13) as usize],
        rank: Rank::new(id % 13 + 1),
    }
}

/// Convert a card set to the bit layout used by MARJJ's Java host.
#[must_use]
pub(super) fn eaai_bits(cards: Hand) -> u64 {
    cards
        .iter()
        .fold(0, |bits, card| bits | 1_u64 << eaai_id(card))
}

/// Cards in MARJJ/EAAI order rather than this crate's C, D, H, S order.
#[must_use]
pub(super) fn ordered_cards(cards: Hand) -> Vec<Card> {
    (0..52)
        .map(card_from_eaai_id)
        .filter(|&card| cards.contains(card))
        .collect()
}

/// Every meld contained in `hand`, in canonical EAAI-bitset order.
fn applicable_melds(hand: Hand) -> Vec<Meld> {
    let mut melds = Vec::new();

    // A rank with all four suits contains each of its four three-card sets
    // as well as the four-card set, just as GinRummyUtil does.
    for rank in 1..=13 {
        for suit_mask in 0_u8..16 {
            if !matches!(suit_mask.count_ones(), 3 | 4) {
                continue;
            }
            let mut cards = Hand::EMPTY;
            for (index, suit) in EAAI_SUITS.into_iter().enumerate() {
                if suit_mask & (1 << index) != 0 {
                    cards.insert(Card {
                        suit,
                        rank: Rank::new(rank),
                    });
                }
            }
            if cards & hand == cards {
                melds.push(Meld::try_from_cards(cards).expect("a three- or four-card rank set"));
            }
        }
    }

    // Include every sub-run of length at least three.  Distinct ways of
    // splitting a long run are distinct spreads and can expose different
    // layoff endpoints, so they must not be collapsed to their card union.
    for suit in EAAI_SUITS {
        for low in 1..=11 {
            for high in low + 2..=13 {
                let meld = Meld::run(suit, Rank::new(low), Rank::new(high));
                if meld.cards() & hand == meld.cards() {
                    melds.push(meld);
                }
            }
        }
    }

    melds.sort_unstable_by_key(|meld| eaai_bits(meld.cards()));
    melds.dedup();
    melds
}

fn search_partitions(
    hand: Hand,
    applicable: &[Meld],
    start: usize,
    union: Hand,
    chosen: &mut Vec<Meld>,
    best_deadwood: &mut u16,
    best: &mut Vec<Vec<Meld>>,
) {
    let value = pip_sum(hand - union);
    if value < *best_deadwood {
        *best_deadwood = value;
        best.clear();
    }
    if value == *best_deadwood {
        best.push(chosen.clone());
    }

    // Eleven cards cannot contain four disjoint melds.  Keeping the bound
    // explicit also matches Melds' three-slot representation.
    if chosen.len() == 3 {
        return;
    }
    for index in start..applicable.len() {
        let meld = applicable[index];
        if !(union & meld.cards()).is_empty() {
            continue;
        }
        chosen.push(meld);
        search_partitions(
            hand,
            applicable,
            index + 1,
            union | meld.cards(),
            chosen,
            best_deadwood,
            best,
        );
        chosen.pop();
    }
}

/// Enumerate every minimum-deadwood partition of a game-sized hand.
///
/// A meld-free hand has one empty partition.  Within a partition melds are
/// ordered by EAAI card bitset; partitions are ordered lexicographically by
/// those meld bitsets, then by their deadwood-card bitset.  The ordering is a
/// deterministic host replacement for Java `HashSet` iteration.
#[must_use]
pub(crate) fn all_minimum_melds(hand: Hand) -> Vec<Melds> {
    assert!(
        hand.len() <= 11,
        "minimum-meld partitions are only needed for game hands"
    );

    let applicable = applicable_melds(hand);
    let mut raw = Vec::new();
    let mut chosen = Vec::with_capacity(3);
    let mut best_deadwood = u16::MAX;
    search_partitions(
        hand,
        &applicable,
        0,
        Hand::EMPTY,
        &mut chosen,
        &mut best_deadwood,
        &mut raw,
    );

    let mut partitions: Vec<Melds> = raw
        .into_iter()
        .map(|melds| Melds::try_new(hand, &melds).expect("the search emits disjoint hand melds"))
        .collect();
    partitions.sort_by(|left, right| {
        let left_melds: Vec<u64> = left.iter().map(|meld| eaai_bits(meld.cards())).collect();
        let right_melds: Vec<u64> = right.iter().map(|meld| eaai_bits(meld.cards())).collect();
        left_melds
            .cmp(&right_melds)
            .then_with(|| eaai_bits(left.deadwood_cards()).cmp(&eaai_bits(right.deadwood_cards())))
    });
    partitions.dedup();
    partitions
}
