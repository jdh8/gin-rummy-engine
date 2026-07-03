//! A lightweight forward model for Monte Carlo rollouts
//!
//! A [`gin_rummy::Round`] cannot be constructed mid-game, so determinized
//! worlds are rolled out on this crate-private replica of the round rules
//! instead.  It mirrors [`gin_rummy::round`] exactly — the draw/shed cycle,
//! the just-taken-discard restriction, the two-card dead-hand rule, knock
//! and gin settlement with greedy layoffs, undercuts — and the equivalence
//! is guarded by a property test that replays whole rounds through both
//! models (`tests/proptest.rs`).  Any rules change upstream must be
//! mirrored here.

use crate::heuristic::{best_shed, greedy_layoff, improves};
use gin_rummy::{Card, Hand, Meld, Melds, Player, RoundResult, Rules, best_melds, deadwood};

/// Where a rollout resumes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SimPhase {
    /// The initial upcard offer
    Upcard,
    /// The draw of a normal turn
    Draw,
    /// The discard decision of an 11-card hand
    Shed,
}

/// A determinized round state: both hands and the stock order are fixed
#[derive(Debug, Clone)]
pub(crate) struct Sim {
    pub(crate) rules: Rules,
    pub(crate) knock_limit: u8,
    pub(crate) hands: [Hand; 2],
    /// Face-down draw order: the last element is drawn first
    pub(crate) stock: Vec<Card>,
    /// Oldest first: the last element is the top
    pub(crate) pile: Vec<Card>,
    pub(crate) turn: Player,
    pub(crate) phase: SimPhase,
    pub(crate) taken: Option<Card>,
    pub(crate) passes: u8,
    pub(crate) forced_stock: bool,
}

impl Sim {
    /// Take the top of the pile into the acting hand
    pub(crate) fn take_discard(&mut self) {
        let card = self.pile.pop().expect("a draw decision implies a pile");
        self.hands[self.turn as usize].insert(card);
        self.taken = Some(card);
        self.phase = SimPhase::Shed;
    }

    /// Decline the upcard; the second pass forces the non-dealer's stock
    /// draw
    pub(crate) fn pass(&mut self) {
        self.passes += 1;
        self.turn = self.turn.opponent();
        if self.passes == 2 {
            self.phase = SimPhase::Draw;
            self.forced_stock = true;
        }
    }

    /// Draw the top of the stock into the acting hand
    pub(crate) fn draw_stock(&mut self) {
        let card = self
            .stock
            .pop()
            .expect("the dead-hand rule keeps the stock non-empty");
        self.hands[self.turn as usize].insert(card);
        self.forced_stock = false;
        self.phase = SimPhase::Shed;
    }

    /// Discard a card, ending the turn; `Some` when the dead-hand rule
    /// finishes the round
    pub(crate) fn discard(&mut self, card: Card) -> Option<RoundResult> {
        self.hands[self.turn as usize].remove(card);
        self.pile.push(card);
        self.taken = None;

        if self.stock.len() == 2 {
            return Some(RoundResult::Dead);
        }
        self.turn = self.turn.opponent();
        self.phase = SimPhase::Draw;
        None
    }

    /// Discard a card and knock with the given arrangement, settling the
    /// round: gin ends it immediately, otherwise the defender lays off
    /// greedily and the deadwood difference (or undercut) decides
    pub(crate) fn knock(mut self, card: Card, melds: Melds) -> RoundResult {
        let knocker = self.turn;
        self.hands[knocker as usize].remove(card);
        let knocker_deadwood = melds.deadwood();
        let defender = knocker.opponent();

        if knocker_deadwood == 0 {
            return RoundResult::Gin {
                winner: knocker,
                deadwood: deadwood(self.hands[defender as usize]),
            };
        }

        let mut spread: Vec<Meld> = melds.iter().collect();
        while let Some((laid, index)) =
            greedy_layoff(self.hands[defender as usize], spread.iter().copied())
        {
            spread[index] = spread[index]
                .extended(laid)
                .expect("the greedy layoff only proposes legal extensions");
            self.hands[defender as usize].remove(laid);
        }

        let defender_deadwood = deadwood(self.hands[defender as usize]);
        let undercut = defender_deadwood < knocker_deadwood
            || (defender_deadwood == knocker_deadwood && self.rules.undercut_on_tie);
        if undercut {
            RoundResult::Undercut {
                winner: defender,
                margin: knocker_deadwood - defender_deadwood,
            }
        } else {
            RoundResult::Knock {
                winner: knocker,
                margin: defender_deadwood - knocker_deadwood,
            }
        }
    }

    /// Declare big gin, ending the round
    pub(crate) fn big_gin(self) -> RoundResult {
        RoundResult::BigGin {
            winner: self.turn,
            deadwood: deadwood(self.hands[self.turn.opponent() as usize]),
        }
    }

    /// A forward model of a fresh deal, mirroring
    /// [`Round::from_deal`](gin_rummy::Round::from_deal)
    #[cfg(test)]
    fn from_deal(
        rules: Rules,
        dealer: Player,
        hands: [Hand; 2],
        upcard: Card,
        stock: Vec<Card>,
    ) -> Self {
        Self {
            knock_limit: rules.knock_limit,
            rules,
            hands,
            stock,
            pile: vec![upcard],
            turn: dealer.opponent(),
            phase: SimPhase::Upcard,
            taken: None,
            passes: 0,
            forced_stock: false,
        }
    }

    /// Play the round out with the knowledge-free greedy policy on both
    /// seats
    pub(crate) fn rollout(mut self) -> RoundResult {
        loop {
            let hand = self.hands[self.turn as usize];
            match self.phase {
                SimPhase::Upcard => {
                    let top = *self.pile.last().expect("the upcard offer has an upcard");
                    if improves(hand, top) {
                        self.take_discard();
                    } else {
                        self.pass();
                    }
                }
                SimPhase::Draw => {
                    let top = *self.pile.last().expect("the pile is never empty on a draw");
                    if !self.forced_stock && improves(hand, top) {
                        self.take_discard();
                    } else {
                        self.draw_stock();
                    }
                }
                SimPhase::Shed => {
                    if deadwood(hand) == 0 && self.rules.big_gin_bonus.is_some() {
                        return self.big_gin();
                    }
                    let (card, rest) = best_shed(hand, self.taken);
                    if rest <= self.knock_limit {
                        return self.knock(card, best_melds(hand - card.into()));
                    }
                    if let Some(result) = self.discard(card) {
                        return result;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Sim, SimPhase};
    use crate::{HeuristicBot, HeuristicConfig, play_round};
    use gin_rummy::{Card, Hand, Player, Rank, Round, Rules, Suit};
    use proptest::prelude::*;

    /// All 52 cards in a fixed order
    fn full_deck() -> Vec<Card> {
        Hand::ALL.iter().collect()
    }

    /// A [`HeuristicBot`] that plays exactly the rollout policy: no danger
    /// weighting, knock whenever the rules allow
    fn rollout_bot() -> HeuristicBot {
        HeuristicBot::with_config(HeuristicConfig {
            knock_threshold: u8::MAX,
            safety_weight: 0,
        })
    }

    /// The forward model and the real [`Round`] must agree on every greedy
    /// self-play game: same deal in, same result out.  This is the guard on
    /// duplicating the round rules in [`Sim`].
    #[test]
    fn sim_matches_round_on_greedy_selfplay() {
        fn check(deck: &[Card], rules: Rules, dealer: Player) {
            let hands = [
                deck[..10].iter().copied().collect::<Hand>(),
                deck[10..20].iter().copied().collect::<Hand>(),
            ];
            let upcard = deck[20];
            let stock = deck[21..].to_vec();

            let sim = Sim::from_deal(rules, dealer, hands, upcard, stock.clone());
            let round = Round::from_deal(rules, dealer, hands, upcard, stock)
                .expect("a permutation of the deck deals cleanly");
            let result = play_round(round, [&mut rollout_bot(), &mut rollout_bot()])
                .expect("greedy bots play legally");
            assert_eq!(sim.rollout(), result);
        }

        proptest!(|(deck in Just(full_deck()).prop_shuffle(), preset in 0..3, seat in 0..2)| {
            let rules = [Rules::new(), Rules::classic(), Rules::palace()][preset as usize];
            let dealer = Player::ALL[seat as usize];
            check(&deck, rules, dealer);
        });
    }

    /// A hand-scripted knock with layoffs settles the same way in both
    /// models
    #[test]
    fn knock_settlement_matches_round() {
        // Knocker (11 cards mid-turn): three runs plus ♠2 ♠9; shedding the
        // ♠9 knocks with 2 deadwood.
        let knocker: Hand = "A23.456.JQK.29".parse().expect("valid hand");
        // Defender: ♣4 lays off onto the ♣A23 run; ♦8 ♦9 stay deadwood.
        let defender: Hand = "4TJQ.89.789T.".parse().expect("valid hand");
        assert_eq!((knocker.len(), defender.len()), (11, 10));

        let mut deck = full_deck();
        deck.retain(|&card| !knocker.contains(card) && !defender.contains(card));
        let upcard = deck[0];
        let stock = deck[1..].to_vec();

        let sim = Sim {
            rules: Rules::default(),
            knock_limit: 10,
            hands: [knocker, defender],
            stock: stock.clone(),
            pile: vec![upcard],
            turn: Player::One,
            phase: SimPhase::Shed,
            taken: None,
            passes: 0,
            forced_stock: false,
        };
        let shed = Card {
            suit: Suit::Spades,
            rank: Rank::new(9),
        };
        let melds = gin_rummy::best_melds(knocker - shed.into());
        let expected = {
            // Round needs 10-card hands pre-draw; give the knocker its
            // 11th card by drawing the scripted stock top.
            let mut stock = stock;
            let eleventh = shed;
            let ten = knocker - eleventh.into();
            stock.push(eleventh);
            let mut round = Round::from_deal(
                Rules::default(),
                Player::Two,
                [ten, defender],
                upcard,
                stock,
            )
            .expect("a disjoint deal");
            round.pass().expect("player one passes");
            round.pass().expect("player two passes");
            round
                .draw_stock()
                .expect("player one draws the scripted top");
            round.knock(shed, melds).expect("nine deadwood knocks");
            while let Some((card, index)) =
                crate::heuristic::greedy_layoff(round.hand(Player::Two), round.spread())
            {
                round.lay_off(card, index).expect("a legal layoff");
            }
            round.finish_layoffs().expect("settles")
        };
        assert_eq!(sim.knock(shed, melds), expected);
    }
}
