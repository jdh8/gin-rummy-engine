//! The [`Strategy`] trait: a decision procedure for one seat

use crate::{DrawAction, Layoff, TurnAction, UpcardAction, View};

/// A decision procedure for one seat of gin rummy
///
/// Every method receives a [`View`] restricted to the information the seat
/// may legally see.  Methods take `&mut self` so strategies can keep state —
/// an internal random number generator, an opponent model — and the trait is
/// object-safe, so the driver works with `&mut dyn Strategy`.
///
/// A strategy never applies its decisions itself; the [`Table`] driver
/// validates and applies them, rejecting illegal choices as
/// [`EngineError::IllegalAction`].
///
/// [`Table`]: crate::Table
/// [`EngineError::IllegalAction`]: crate::EngineError::IllegalAction
pub trait Strategy {
    /// Take or pass the initial upcard
    fn offer_upcard(&mut self, view: &View<'_>) -> UpcardAction;

    /// Draw from the stock or the discard pile
    ///
    /// Not consulted on the forced stock draw after both players pass the
    /// upcard — the driver draws from the stock directly, so implementations
    /// may treat [`DrawAction::TakeDiscard`] as always available.
    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction;

    /// Discard, knock, or declare big gin
    ///
    /// The drawn card is already in [`View::hand`], which holds 11 cards;
    /// [`View::taken_discard`] is the card that may not be shed this turn.
    fn play_turn(&mut self, view: &View<'_>) -> TurnAction;

    /// Lay one card off onto the knocker's spread, or `None` to finish
    ///
    /// Called repeatedly with a refreshed view — the spread grows with every
    /// accepted layoff — until the strategy returns `None`.
    fn choose_layoff(&mut self, view: &View<'_>) -> Option<Layoff>;

    /// A display name for tournament output
    fn name(&self) -> &str {
        "unnamed"
    }
}
