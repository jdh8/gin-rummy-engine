//! The [`View`]: what one seat may legally see of a round
//!
//! [`Round`] exposes the whole position — both hands and the stock order —
//! and leaves information hygiene to its consumers.  This module is that
//! hygiene: a [`View`] borrows the round privately and re-exposes only the
//! whitelist of legally visible information, paired with the per-seat
//! [`Knowledge`] that the driver accumulates and that no round snapshot can
//! recover (which discards the opponent took, what they declined, whether
//! this turn's draw is forced from the stock).

use gin_rummy::{Card, Hand, Meld, Melds, Phase, Player, Round, Rules, best_melds, deadwood};

/// What a seat has learned beyond the public table state
///
/// Owned and updated by the driver as actions are applied; a `Round`
/// snapshot alone cannot reconstruct it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Knowledge {
    /// Cards known to be in the opponent's hand: taken from the pile and
    /// not yet shed or laid off
    pub(crate) opponent_known: Hand,
    /// Every card the opponent has discarded, whether or not it is still
    /// in the pile
    pub(crate) opponent_shed: Hand,
    /// Upcards the opponent declined during the upcard phase
    pub(crate) opponent_passed: Hand,
    /// The card this seat took from the pile this turn, which may not be
    /// shed until the next turn
    pub(crate) taken_discard: Option<Card>,
    /// The next draw must come from the stock (both players passed the
    /// upcard)
    pub(crate) forced_stock: bool,
}

/// What one seat may legally see of a round
///
/// The wrapped [`Round`] is private, and these accessors are the whitelist:
/// the opponent's hand and the stock order are structurally unreachable.
pub struct View<'a> {
    round: &'a Round,
    seat: Player,
    know: &'a Knowledge,
}

impl<'a> View<'a> {
    pub(crate) const fn new(round: &'a Round, seat: Player, know: &'a Knowledge) -> Self {
        Self { round, seat, know }
    }

    /// The seat this view belongs to
    #[must_use]
    pub const fn seat(&self) -> Player {
        self.seat
    }

    /// The scoring rules of the round
    #[must_use]
    pub const fn rules(&self) -> &'a Rules {
        self.round.rules()
    }

    /// The dealer of the round
    #[must_use]
    pub const fn dealer(&self) -> Player {
        self.round.dealer()
    }

    /// The knock limit in effect
    #[must_use]
    pub const fn knock_limit(&self) -> u8 {
        self.round.knock_limit()
    }

    /// The current phase of the round
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.round.phase()
    }

    /// The discard pile, oldest first — the last card is the top
    #[must_use]
    pub fn discard_pile(&self) -> &'a [Card] {
        self.round.discard_pile()
    }

    /// The top of the discard pile
    #[must_use]
    pub fn upcard(&self) -> Option<Card> {
        self.round.discard_pile().last().copied()
    }

    /// How many cards remain in the stock — the order is never visible
    #[must_use]
    pub fn stock_len(&self) -> usize {
        self.round.stock().len()
    }

    /// The knocker's spread, extended by any layoffs so far
    ///
    /// Empty before a knock.  Enumeration order gives the meld indices that
    /// [`Layoff::meld`](crate::Layoff::meld) addresses.
    pub fn spread(&self) -> impl Iterator<Item = Meld> + 'a {
        self.round.spread()
    }

    /// The knocker, if the round has reached a knock
    #[must_use]
    pub const fn knocker(&self) -> Option<Player> {
        self.round.knocker()
    }

    /// This seat's hand
    #[must_use]
    pub const fn hand(&self) -> Hand {
        self.round.hand(self.seat)
    }

    /// The card this seat took from the pile this turn, which may not be
    /// shed until the next turn
    #[must_use]
    pub const fn taken_discard(&self) -> Option<Card> {
        self.know.taken_discard
    }

    /// Whether taking the top of the discard pile is available
    ///
    /// True at the upcard offer and on a normal draw, false on the forced
    /// stock draw after both players pass the upcard.
    #[must_use]
    pub const fn can_take_discard(&self) -> bool {
        match self.round.phase() {
            Phase::Upcard => true,
            Phase::Draw => !self.know.forced_stock,
            Phase::Discard | Phase::Layoff | Phase::Finished => false,
        }
    }

    /// Cards known to be in the opponent's hand: taken from the pile and
    /// not yet shed or laid off
    #[must_use]
    pub const fn opponent_known(&self) -> Hand {
        self.know.opponent_known
    }

    /// Every card the opponent has discarded, whether or not it is still in
    /// the pile
    #[must_use]
    pub const fn opponent_shed(&self) -> Hand {
        self.know.opponent_shed
    }

    /// Upcards the opponent declined during the upcard phase
    #[must_use]
    pub const fn opponent_passed(&self) -> Hand {
        self.know.opponent_passed
    }

    /// How many cards the opponent holds — 10, or 11 mid-turn
    #[must_use]
    pub const fn opponent_hand_len(&self) -> usize {
        self.round.hand(self.seat.opponent()).len()
    }

    /// The cards this seat cannot locate: the whole deck minus its own
    /// hand, the discard pile, the opponent's known cards, and the spread
    ///
    /// Exactly the cards a determinizing bot must distribute between the
    /// stock and the hidden part of the opponent's hand.  Until a knock
    /// reveals the spread,
    /// `unseen().len() == stock_len() + opponent_hand_len() -
    /// opponent_known().len()`; afterwards the spread also counts as seen.
    #[must_use]
    pub fn unseen(&self) -> Hand {
        let seen = self
            .round
            .discard_pile()
            .iter()
            .fold(self.hand() | self.know.opponent_known, |acc, &card| {
                acc | card.into()
            });
        let seen = self
            .round
            .spread()
            .fold(seen, |acc, meld| acc | meld.cards());
        Hand::ALL - seen
    }

    /// The minimum deadwood of this seat's hand
    #[must_use]
    pub fn deadwood(&self) -> u8 {
        deadwood(self.hand())
    }

    /// A deadwood-minimizing arrangement of this seat's hand
    #[must_use]
    pub fn best_melds(&self) -> Melds {
        best_melds(self.hand())
    }
}
