//! The decisions a [`Strategy`](crate::Strategy) can make
//!
//! Each phase of a round has its own action type, so an action that is
//! structurally illegal for its decision point cannot be expressed.

use gin_rummy::{Card, Melds};

/// Response to the initial upcard offer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpcardAction {
    /// Take the upcard into hand
    Take,
    /// Decline the upcard, passing the offer on
    Pass,
}

/// Where to draw at the start of a normal turn
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawAction {
    /// Draw the hidden top of the stock
    Stock,
    /// Take the top of the discard pile
    TakeDiscard,
}

/// How to end a turn from an 11-card hand
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnAction {
    /// Shed a card and pass the turn
    Discard(Card),
    /// Shed a card and knock, spreading the given arrangement
    ///
    /// The arrangement decides what the defender may lay off, so it is
    /// chosen explicitly; `best_melds(hand - discard.into())` recovers the
    /// deadwood-minimizing choice.
    Knock {
        /// The card to shed
        discard: Card,
        /// The arrangement of the remaining ten cards to spread
        melds: Melds,
    },
    /// Declare big gin with all 11 cards arranged and no discard
    BigGin(Melds),
}

/// One layoff onto the knocker's spread
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Layoff {
    /// The card to lay off
    pub card: Card,
    /// The index of the target meld in [`View::spread`](crate::View::spread)
    /// enumeration order
    ///
    /// Indices are stable across layoffs, so chained run extensions address
    /// the same meld.
    pub meld: usize,
}
