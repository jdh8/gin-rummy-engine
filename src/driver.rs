//! The [`Table`] driver: applies strategy decisions to a round
//!
//! The driver owns the [`Round`] and both seats' [`Knowledge`], asks the
//! acting strategy for one decision at a time, validates it, applies it, and
//! keeps every observer's knowledge current — so information hygiene holds
//! by construction for any [`Strategy`].

use crate::view::Knowledge;
use crate::{DrawAction, Strategy, TurnAction, UpcardAction, View};
use gin_rummy::round::RoundError;
use gin_rummy::{Phase, Player, Round, RoundResult};
use thiserror::Error;

/// An error while driving a round
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EngineError {
    /// A strategy chose an action the round rejected
    #[error("{seat} chose an illegal action")]
    IllegalAction {
        /// The seat whose strategy misbehaved
        seat: Player,
        /// The round's rejection
        #[source]
        source: RoundError,
    },
}

/// A round in progress, with per-seat knowledge tracking
///
/// [`Table::round`] intentionally exposes the full position — user
/// interfaces and loggers need it — but strategies only ever receive the
/// [`View`]s handed to them by [`Table::step`].
#[derive(Debug, Clone)]
pub struct Table {
    round: Round,
    knowledge: [Knowledge; 2],
    scores: [u16; 2],
}

impl Table {
    /// Wrap a freshly dealt round
    ///
    /// The round must still be in the upcard phase: knowledge accumulated
    /// during earlier play cannot be reconstructed from a mid-game round.
    ///
    /// The game score defaults to level (both seats at zero), so a
    /// standalone round reports [`game_scores`](View::game_scores) of
    /// `[0, 0]` and a [`game_margin`](View::game_margin) of zero.  Set it
    /// with [`Table::scores`] when driving a round within a game.
    #[must_use]
    pub fn new(round: Round) -> Self {
        debug_assert_eq!(round.phase(), Phase::Upcard);
        Self {
            round,
            knowledge: [Knowledge::default(); 2],
            scores: [0; 2],
        }
    }

    /// Set the running game totals, indexed by [`Player`]
    ///
    /// Each seat's [`View`] then reports the seat-relative
    /// [`game_scores`](View::game_scores) and
    /// [`game_margin`](View::game_margin), letting score-aware strategies
    /// bank a lead or gamble from behind.
    #[must_use]
    pub const fn scores(mut self, scores: [u16; 2]) -> Self {
        self.scores = scores;
        self
    }

    /// Shuffle and deal a fresh round onto a new table
    #[cfg(feature = "rand")]
    #[must_use]
    pub fn deal(
        rules: gin_rummy::Rules,
        dealer: Player,
        rng: &mut (impl rand::Rng + ?Sized),
    ) -> Self {
        Self::new(Round::deal(rules, dealer, rng))
    }

    /// The underlying round, fully visible
    #[must_use]
    pub const fn round(&self) -> &Round {
        &self.round
    }

    /// The player to act, or `None` when the round is finished
    #[must_use]
    pub const fn turn(&self) -> Option<Player> {
        self.round.turn()
    }

    /// The legally visible information for one seat
    #[must_use]
    pub const fn view(&self, seat: Player) -> View<'_> {
        let scores = [
            self.scores[seat as usize],
            self.scores[seat.opponent() as usize],
        ];
        View::new(&self.round, seat, &self.knowledge[seat as usize], scores)
    }

    /// Ask `strategy` — which must belong to the seat to act — for one
    /// decision and apply it
    ///
    /// Returns the result once the round finishes, `None` while it
    /// continues.  On the forced stock draw after both players pass the
    /// upcard, the strategy is not consulted; the draw is applied directly.
    ///
    /// # Errors
    ///
    /// [`EngineError::IllegalAction`] when the round rejects the strategy's
    /// choice.  The table is left unchanged, so an interactive caller may
    /// retry the same decision.
    pub fn step(
        &mut self,
        strategy: &mut dyn Strategy,
    ) -> Result<Option<RoundResult>, EngineError> {
        let Some(seat) = self.round.turn() else {
            return Ok(self.round.result());
        };
        let reject = |source| EngineError::IllegalAction { seat, source };

        match self.round.phase() {
            Phase::Upcard => {
                let top = self.round.discard_pile().last().copied();
                match strategy.offer_upcard(&self.view(seat)) {
                    UpcardAction::Take => {
                        let card = self.round.take_discard().map_err(reject)?;
                        self.knowledge[seat as usize].taken_discard = Some(card);
                        self.knowledge[seat.opponent() as usize]
                            .opponent_known
                            .insert(card);
                    }
                    UpcardAction::Pass => {
                        self.round.pass().map_err(reject)?;
                        if let Some(card) = top {
                            self.knowledge[seat.opponent() as usize]
                                .opponent_passed
                                .insert(card);
                        }
                        // The second pass forces the non-dealer's stock draw.
                        if self.round.phase() == Phase::Draw {
                            self.knowledge[self.round.non_dealer() as usize].forced_stock = true;
                        }
                    }
                }
            }
            Phase::Draw => {
                let action = if self.knowledge[seat as usize].forced_stock {
                    DrawAction::Stock
                } else {
                    strategy.choose_draw(&self.view(seat))
                };
                match action {
                    DrawAction::Stock => {
                        self.round.draw_stock().map_err(reject)?;
                        self.knowledge[seat as usize].forced_stock = false;
                    }
                    DrawAction::TakeDiscard => {
                        let card = self.round.take_discard().map_err(reject)?;
                        self.knowledge[seat as usize].taken_discard = Some(card);
                        self.knowledge[seat.opponent() as usize]
                            .opponent_known
                            .insert(card);
                    }
                }
            }
            Phase::Discard => {
                let shed = match strategy.play_turn(&self.view(seat)) {
                    TurnAction::Discard(card) => {
                        self.round.discard(card).map_err(reject)?;
                        Some(card)
                    }
                    TurnAction::Knock { discard, melds } => {
                        self.round.knock(discard, melds).map_err(reject)?;
                        Some(discard)
                    }
                    TurnAction::BigGin(melds) => {
                        self.round.declare_big_gin(melds).map_err(reject)?;
                        None
                    }
                };
                self.knowledge[seat as usize].taken_discard = None;
                if let Some(card) = shed {
                    let observer = &mut self.knowledge[seat.opponent() as usize];
                    observer.opponent_known.remove(card);
                    observer.opponent_shed.insert(card);
                }
            }
            Phase::Layoff => match strategy.choose_layoff(&self.view(seat)) {
                Some(layoff) => {
                    self.round
                        .lay_off(layoff.card, layoff.meld)
                        .map_err(reject)?;
                    // The card is public on the spread now, not in the hand.
                    self.knowledge[seat.opponent() as usize]
                        .opponent_known
                        .remove(layoff.card);
                }
                None => {
                    self.round.finish_layoffs().map_err(reject)?;
                }
            },
            Phase::Finished => {}
        }
        Ok(self.round.result())
    }

    /// Drive the round to completion, one strategy per seat
    ///
    /// # Errors
    ///
    /// [`EngineError::IllegalAction`] when a strategy's choice is rejected.
    pub fn play(&mut self, strategies: [&mut dyn Strategy; 2]) -> Result<RoundResult, EngineError> {
        loop {
            let Some(seat) = self.round.turn() else {
                return Ok(self.round.result().expect("a turnless round is finished"));
            };
            if let Some(result) = self.step(&mut *strategies[seat as usize])? {
                return Ok(result);
            }
        }
    }
}

/// Play one round to completion, one strategy per seat, indexed by
/// [`Player`]
///
/// # Errors
///
/// [`EngineError::IllegalAction`] when a strategy's choice is rejected.
pub fn play_round(
    round: Round,
    strategies: [&mut dyn Strategy; 2],
) -> Result<RoundResult, EngineError> {
    Table::new(round).play(strategies)
}

/// Deal and play rounds until the game is over, returning the settled score
///
/// # Errors
///
/// [`EngineError::IllegalAction`] when a strategy's choice is rejected.
/// The game keeps the rounds recorded so far.
#[cfg(feature = "rand")]
pub fn play_game(
    game: &mut gin_rummy::Game,
    strategies: [&mut dyn Strategy; 2],
    rng: &mut (impl rand::Rng + ?Sized),
) -> Result<gin_rummy::FinalScore, EngineError> {
    let [one, two] = strategies;
    while !game.is_over() {
        let scores = [game.score(Player::One), game.score(Player::Two)];
        let mut table = Table::new(game.deal(rng)).scores(scores);
        let result = table.play([&mut *one, &mut *two])?;
        game.record(result)
            .expect("a result produced by the round it was dealt for records cleanly");
    }
    Ok(game.final_score().expect("a game that is over settles"))
}
