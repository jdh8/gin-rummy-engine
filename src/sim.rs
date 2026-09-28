//! A lightweight forward model for Monte Carlo rollouts
//!
//! A [`gin_rummy::Round`] cannot be constructed mid-game, so determinized
//! worlds are rolled out on this crate-private replica of the round rules
//! instead.  It mirrors [`gin_rummy::round`] exactly — the draw/shed cycle,
//! the just-taken-discard restriction, the two-card dead-hand rule, knock
//! and gin settlement with greedy layoffs, undercuts — and the equivalence
//! is guarded by a property test that replays whole rounds through both
//! models (`sim_matches_round_on_greedy_selfplay` in this module's
//! tests).  Any rules change upstream must be mirrored here.

use crate::heuristic::{best_shed, greedy_layoff, improving_melds, joins_a_meld};
use gin_rummy::{
    Card, Hand, Meld, Melds, Player, RoundResult, Rules, best_melds, deadwood, pip_sum,
};

/// How the forward model plays one seat during a rollout
///
/// The default — knock at the first legal chance, draw on any strict
/// improvement — is the knowledge-free greedy policy the rollout has
/// always played; [`McConfig`](crate::McConfig) maps its rollout knobs
/// onto per-seat values through [`MonteCarloBot::sim`](crate::MonteCarloBot).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SeatPolicy {
    /// Knock at residual deadwood ≤ `min(knock_limit, knock_threshold)`;
    /// `u8::MAX` knocks whenever the rules allow
    pub(crate) knock_threshold: u8,
    /// Take the pile card only when it lands in an immediate meld
    /// ([`joins_a_meld`], the EAAI baseline's rule) instead of on any
    /// strict deadwood improvement ([`improving_melds`])
    pub(crate) meld_only_draw: bool,
}

impl Default for SeatPolicy {
    fn default() -> Self {
        Self {
            knock_threshold: u8::MAX,
            meld_only_draw: false,
        }
    }
}

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
    /// How [`rollout`](Self::rollout) plays each seat; the default plays
    /// the historical knowledge-free greedy on both
    pub(crate) policies: [SeatPolicy; 2],
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
    ///
    /// The arrangement may still contain the discarded card as deadwood.
    pub(crate) fn knock(mut self, card: Card, melds: Melds) -> RoundResult {
        debug_assert!(!melds.melded().contains(card));
        let knocker = self.turn;
        self.hands[knocker as usize].remove(card);
        let knocker_deadwood = pip_sum(melds.deadwood_cards() - card.into()) as u8;
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
    ///
    /// Test-only, and crate-visible so that [`crate::value`] can resample the
    /// greedy self-play its baked outcome models are measured from.
    #[cfg(test)]
    pub(crate) fn from_deal(
        rules: Rules,
        dealer: Player,
        hands: [Hand; 2],
        upcard: Card,
        stock: Vec<Card>,
    ) -> Self {
        Self {
            // Oklahoma reads the limit off the opening upcard.
            knock_limit: rules.knock_limit_for(upcard),
            rules,
            hands,
            stock,
            pile: vec![upcard],
            turn: dealer.opponent(),
            phase: SimPhase::Upcard,
            taken: None,
            passes: 0,
            forced_stock: false,
            policies: [SeatPolicy::default(); 2],
        }
    }

    /// The solved draw when the acting seat's policy takes the pile top.
    fn take_melds(&self, hand: Hand, top: Card, before: Option<u8>) -> Option<Melds> {
        if self.policies[self.turn as usize].meld_only_draw {
            joins_a_meld(hand, top).then(|| best_melds(hand | top.into()))
        } else {
            improving_melds(hand, top, before)
        }
    }

    /// Play the round out, each seat following its [`SeatPolicy`] over the
    /// knowledge-free greedy core
    ///
    /// Under the default policies both seats knock at the first legal
    /// chance.  Raising the model's fidelity instead — holding *both*
    /// seats to the shipped heuristic's tuned knock threshold of 4 —
    /// measured clearly weaker (−6 and −8 points of decisive win rate on
    /// two 10 000-round seeds and −11 points over 300 games, mc:64 head
    /// to head): the urgent knocker is the threat model that prices
    /// deadwood risk correctly, and a patient forward model plays
    /// complacent.  The per-seat policies exist to test the *asymmetric*
    /// cases that finding does not cover.
    pub(crate) fn rollout(self) -> RoundResult {
        self.rollout_observed(|_| ())
    }

    /// [`rollout`](Self::rollout), calling `probe` on every state the round
    /// passes through
    ///
    /// The probe is how the calibration curves in [`crate::mc`] are measured:
    /// they need the hidden hand's deadwood as the round develops, which the
    /// result alone cannot report.  `rollout` is this function with an inert
    /// probe, so the two cannot drift.
    pub(crate) fn rollout_observed(mut self, mut probe: impl FnMut(&Self)) -> RoundResult {
        // A seat's ten-card hand stays unchanged between its shed and draw.
        // Local caches also allow callers to resume from any phase.
        let mut deadwood = [None; 2];
        let mut drawn_melds = None;
        loop {
            probe(&self);
            let seat = self.turn as usize;
            let hand = self.hands[seat];
            match self.phase {
                SimPhase::Upcard => {
                    let top = *self.pile.last().expect("the upcard offer has an upcard");
                    drawn_melds = self.take_melds(hand, top, deadwood[seat]);
                    if drawn_melds.is_some() {
                        self.take_discard();
                    } else {
                        self.pass();
                    }
                }
                SimPhase::Draw => {
                    let top = *self.pile.last().expect("the pile is never empty on a draw");
                    drawn_melds = if self.forced_stock {
                        None
                    } else {
                        self.take_melds(hand, top, deadwood[seat])
                    };
                    if drawn_melds.is_some() {
                        self.take_discard();
                    } else {
                        self.draw_stock();
                    }
                }
                SimPhase::Shed => {
                    let melds = drawn_melds.take().unwrap_or_else(|| best_melds(hand));
                    if self.rules.big_gin_bonus.is_some() && melds.deadwood() == 0 {
                        return self.big_gin();
                    }
                    let (card, rest) = best_shed(melds, self.taken);
                    let threshold = self.policies[self.turn as usize].knock_threshold;
                    if rest <= self.knock_limit.min(threshold) {
                        let melds = if melds.deadwood_cards().contains(card) {
                            melds
                        } else {
                            best_melds(hand - card.into())
                        };
                        return self.knock(card, melds);
                    }
                    deadwood[seat] = Some(rest);
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
    use gin_rummy::{Card, Hand, OklahomaAce, Player, Rank, Round, Rules, Suit};
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
            score_awareness: 0,
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

        proptest!(|(deck in Just(full_deck()).prop_shuffle(), preset in 0..5, seat in 0..2)| {
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
            ][preset as usize];
            let dealer = Player::ALL[seat as usize];
            check(&deck, rules, dealer);
        });
    }

    /// Reuse preserves every observed state, including non-default policies
    /// and rollouts resumed mid-round with empty caches.
    #[test]
    fn cached_rollout_matches_uncached_play() {
        proptest!(|(
            deck in Just(full_deck()).prop_shuffle(),
            preset in 0..3_usize,
            thresholds in prop::array::uniform2(0..=10_u8),
            meld_only in prop::array::uniform2(any::<bool>()),
            start in any::<usize>(),
        )| {
            let hands = [
                deck[..10].iter().copied().collect::<Hand>(),
                deck[10..20].iter().copied().collect::<Hand>(),
            ];
            let mut sim = Sim::from_deal(
                [Rules::new(), Rules::classic(), Rules::palace()][preset],
                Player::One,
                hands,
                deck[20],
                deck[21..].to_vec(),
            );
            sim.policies = std::array::from_fn(|seat| super::SeatPolicy {
                knock_threshold: thresholds[seat],
                meld_only_draw: meld_only[seat],
            });
            let mut states = Vec::new();
            let expected = loop {
                states.push(sim.clone());
                let hand = sim.hands[sim.turn as usize];
                let policy = sim.policies[sim.turn as usize];
                match sim.phase {
                    SimPhase::Upcard | SimPhase::Draw => {
                        let top = *sim.pile.last().expect("a pile on a draw");
                        let take = !sim.forced_stock && if policy.meld_only_draw {
                            crate::heuristic::joins_a_meld(hand, top)
                        } else {
                            crate::heuristic::improves(hand, top)
                        };
                        if take {
                            sim.take_discard();
                        } else if sim.phase == SimPhase::Upcard {
                            sim.pass();
                        } else {
                            sim.draw_stock();
                        }
                    }
                    SimPhase::Shed => {
                        let melds = gin_rummy::best_melds(hand);
                        if sim.rules.big_gin_bonus.is_some() && melds.deadwood() == 0 {
                            break sim.big_gin();
                        }
                        let (card, rest) = crate::heuristic::best_shed(melds, sim.taken);
                        if rest <= sim.knock_limit.min(policy.knock_threshold) {
                            break sim.knock(card, gin_rummy::best_melds(hand - card.into()));
                        }
                        if let Some(result) = sim.discard(card) {
                            break result;
                        }
                    }
                }
            };
            let mut states = states[start % states.len()..].iter();
            let resumed = states.clone().next().expect("at least one state").clone();
            let actual = resumed.rollout_observed(|actual| {
                let expected = states.next().expect("no extra rollout states");
                assert_eq!(actual.hands, expected.hands);
                assert_eq!(actual.stock, expected.stock);
                assert_eq!(actual.pile, expected.pile);
                assert_eq!(actual.turn, expected.turn);
                assert_eq!(actual.phase, expected.phase);
                assert_eq!(actual.taken, expected.taken);
                assert_eq!(actual.passes, expected.passes);
                assert_eq!(actual.forced_stock, expected.forced_stock);
            });
            prop_assert!(states.next().is_none());
            prop_assert_eq!(actual, expected);
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
            policies: [super::SeatPolicy::default(); 2],
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
        assert_eq!(sim.clone().knock(shed, melds), expected);
        assert_eq!(sim.knock(shed, gin_rummy::best_melds(knocker)), expected);
    }
}
