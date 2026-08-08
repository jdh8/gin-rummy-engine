//! Independent native host adaptation of the paper's Gold Standard Agent.
//!
//! The upstream name describes an exact meld solver wrapped in a fixed
//! reactive heuristic; it does not claim that the full-game policy is
//! game-theoretically optimal.  This implementation preserves that policy's
//! representable draw, discard, and knock choices while adapting missing
//! phases to this engine's rules.
//!
//! Behavioral reference: `gold_standard_agent.py` at commit
//! `3b2f5b7866d27234647c5833497c12ca1a2afde9`, SHA-256
//! `88a5ed62638de8c45c0a679c42cd2b05656b93336af9760905d77af04d1e7bca`,
//! together with <https://arxiv.org/html/2607.06854v1>.

use super::greedy_layoff;
use gin_rummy::{Card, Hand, Suit, best_melds, deadwood};
use gin_rummy_engine::{DrawAction, Layoff, Strategy, TurnAction, UpcardAction, View};

/// Benchmark-only adaptation of the 2026 Gold Standard Agent heuristic.
///
/// This type intentionally is not exported by `gin-rummy-engine`.  The
/// upstream benchmark used different dealing, settlement, and reward
/// semantics; `GoldPaperBot` is a controlled host-engine adaptation, not an
/// execution of that Python agent.
#[derive(Debug, Clone, Copy, Default)]
pub struct GoldPaperBot;

impl GoldPaperBot {
    /// Construct the deterministic paper-policy adaptation.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// RLCard's ascending card id, whose suits are ordered S, H, D, C.
    const fn rlcard_id(card: Card) -> u8 {
        let suit = match card.suit {
            Suit::Spades => 0,
            Suit::Hearts => 1,
            Suit::Diamonds => 2,
            Suit::Clubs => 3,
        };
        suit * 13 + card.rank.get() - 1
    }

    /// The source's comparison key for an ordinary discard.
    pub(crate) const fn ordinary_key(card: Card, rest: u8) -> (u8, u8, u8) {
        (rest, u8::MAX - card.rank.deadwood(), Self::rlcard_id(card))
    }

    /// The source's comparison key for a non-gin knock discard.
    pub(crate) const fn knock_key(card: Card, rest: u8) -> (u8, u8) {
        (rest, Self::rlcard_id(card))
    }

    /// Whether keeping the visible card strictly lowers achievable deadwood.
    pub(crate) fn improves(hand: Hand, top: Card) -> bool {
        let with = hand | top.into();
        let best_take = with
            .iter()
            .filter(|&discard| discard != top)
            .map(|discard| deadwood(with - discard.into()))
            .min()
            .expect("a ten-card hand has a legal discard after taking");
        best_take < deadwood(hand)
    }

    /// The ordinary discard: minimum deadwood, then highest pip card, then
    /// lowest RLCard id.
    pub(crate) fn ordinary_shed(hand: Hand, taken: Option<Card>) -> (Card, u8) {
        hand.iter()
            .filter(|&card| Some(card) != taken)
            .map(|card| (card, deadwood(hand - card.into())))
            .min_by_key(|&(card, rest)| Self::ordinary_key(card, rest))
            .expect("an eleven-card hand has a legal discard")
    }

    /// The non-gin knock discard.  The source omits the ordinary discard's
    /// pip tie-break here and lets ascending RLCard id settle equal deadwood.
    pub(crate) fn knock_shed(hand: Hand, taken: Option<Card>) -> (Card, u8) {
        hand.iter()
            .filter(|&card| Some(card) != taken)
            .map(|card| (card, deadwood(hand - card.into())))
            .min_by_key(|&(card, rest)| Self::knock_key(card, rest))
            .expect("an eleven-card hand has a legal discard")
    }

    /// Select a discard or knock from an eleven-card hand.
    pub(crate) fn turn_action(hand: Hand, taken: Option<Card>, knock_limit: u8) -> TurnAction {
        let (ordinary, ordinary_deadwood) = Self::ordinary_shed(hand, taken);

        // RLCard exposes gin as a dedicated action.  In this host engine a
        // zero-deadwood discard is the equivalent declaration; retain the
        // ordinary discard key exactly for this special case.
        if ordinary_deadwood == 0 {
            return TurnAction::Knock {
                discard: ordinary,
                melds: best_melds(hand - ordinary.into()),
            };
        }

        let (knock, knock_deadwood) = Self::knock_shed(hand, taken);
        if knock_deadwood <= knock_limit.min(10) {
            TurnAction::Knock {
                discard: knock,
                melds: best_melds(hand - knock.into()),
            }
        } else {
            TurnAction::Discard(ordinary)
        }
    }
}

impl Strategy for GoldPaperBot {
    fn offer_upcard(&mut self, view: &View<'_>) -> UpcardAction {
        let top = view.upcard().expect("the upcard offer has an upcard");
        if Self::improves(view.hand(), top) {
            UpcardAction::Take
        } else {
            UpcardAction::Pass
        }
    }

    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction {
        let top = view.upcard().expect("the pile is never empty on a draw");
        if Self::improves(view.hand(), top) {
            DrawAction::TakeDiscard
        } else {
            DrawAction::Stock
        }
    }

    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        Self::turn_action(view.hand(), view.taken_discard(), view.knock_limit())
    }

    fn choose_layoff(&mut self, view: &View<'_>) -> Option<Layoff> {
        greedy_layoff(view.hand(), view.spread())
    }

    fn name(&self) -> &str {
        "gold-paper"
    }
}
