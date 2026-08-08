//! Deterministic benchmark surrogate for the public MARJJ v5 policy.
//!
//! This is an independent native reconstruction of the public v5 policy, not
//! the original Java program.  It retains the published constants and
//! observable policy quirks while adapting callback state to [`View`] and
//! resolving Java collection/random ties reproducibly with an injected RNG.
//!
//! Behavioral reference: public `MARJJ_v5-1.java` at commit
//! `5d1f00c1dff5380021785c8146d039a11efcabc3`, SHA-256
//! `df6d4db2476ea35ee193258eec12f4925e1ea4d0fb703283fea3b1d4f82b9a4f`.
//! That file uses 18, 0.9, and seven future cards; the associated paper
//! describes 20, 0.9, and six.

use super::melds::{all_minimum_melds, eaai_bits, eaai_id, ordered_cards};
use gin_rummy::{Card, Hand, MeldKind, Melds, best_melds, deadwood};
use gin_rummy_engine::{
    DrawAction, HeuristicBot, Layoff, Strategy, TurnAction, UpcardAction, View,
};
use rand::{Rng, RngExt as _};

const MAX_DEADWOOD_DIFFERENCE: u8 = 5;
const INIT_WEIGHT: f64 = 18.0;
const DECAY: f64 = 0.9;
const MAX_CARDS_TO_CONSIDER: usize = 7;

const SEQUENCE_BONUS: f64 = -10.0;
const ADJ_BONUS: f64 = -5.0;
const ADJ2_BONUS: f64 = -3.0;
const SEQUENCE_PENALTY: f64 = 20.0;
const ADJ_PENALTY: f64 = 5.0;
const ADJ2_PENALTY: f64 = 3.0;
const SET_BONUS: f64 = -10.0;
const RANK_BONUS: f64 = -4.0;
const SET_PENALTY: f64 = 20.0;
const RANK_PENALTY: f64 = 4.0;

/// Components of MARJJ's score for one filtered discard candidate.
///
/// Kept crate-visible so the optional source-conformance harness can compare
/// traces without making benchmark diagnostics part of the library API.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DiscardEvaluation {
    pub(crate) card: Card,
    pub(crate) post_deadwood: u8,
    pub(crate) future_mean: f64,
    pub(crate) danger: f64,
    pub(crate) total: f64,
}

/// Benchmark-only surrogate for public MARJJ v5.
///
/// The repository is associated with the team behind `MARJJ_Player`, winner
/// of the 2021 EAAI challenge; it does not establish that this separately
/// named v5 file is the submitted championship implementation.
///
/// The EAAI host called event-reporting methods on both players.  This engine
/// instead provides a knowledge-filtered `View` only when a player acts, so
/// the surrogate folds newly visible information into two legacy masks at
/// every decision.  In particular, cards visibly taken by the opponent stay
/// in MARJJ's opponent mask even after the opponent sheds them; that stale
/// union is an intentional v5 behavior.
#[derive(Debug, Clone)]
pub struct MarjjV5Surrogate<R> {
    rng: R,
    seen: Hand,
    opponent_cards: Hand,
    turns: u32,
    last_hand: Option<Hand>,
    initialized: bool,
}

impl<R: Rng> MarjjV5Surrogate<R> {
    /// Construct a surrogate over an injected RNG; seed it for replayability.
    #[must_use]
    pub const fn new(rng: R) -> Self {
        Self {
            rng,
            seen: Hand::EMPTY,
            opponent_cards: Hand::EMPTY,
            turns: 0,
            last_hand: None,
            initialized: false,
        }
    }

    fn reset(&mut self, view: &View<'_>) {
        self.seen = Hand::EMPTY;
        self.opponent_cards = Hand::EMPTY;
        self.turns = 0;
        self.last_hand = Some(view.hand());
        self.initialized = true;
        self.observe(view);
    }

    /// Fold information corresponding to Java's draw/discard reports into
    /// MARJJ's deliberately lossy masks.  Pile takes are *not* marked seen.
    fn observe(&mut self, view: &View<'_>) {
        self.seen |= view.hand() | view.opponent_shed();
        self.opponent_cards |= view.opponent_known();
    }

    fn observe_offered_card(&mut self, view: &View<'_>) -> Card {
        let top = view.upcard().expect("a draw decision has a face-up card");
        self.seen.insert(top);
        top
    }

    /// Java's helper minimizes deadwood after an unrestricted discard,
    /// including the newly offered card.  The later meld-membership test is
    /// what prevents a take-and-immediately-return action.
    fn deadwood_after_discard(hand: Hand) -> u8 {
        ordered_cards(hand)
            .into_iter()
            .map(|card| deadwood(hand - card.into()))
            .min()
            .expect("an eleven-card hand contains a discard")
    }

    /// Whether the face-up card passes both of MARJJ v5's draw gates.
    #[must_use]
    pub(crate) fn would_take(hand: Hand, top: Card, seen: Hand, opponent_cards: Hand) -> bool {
        let with_top = hand | top.into();
        let Some(selected) = Self::select_spread(with_top, seen, opponent_cards) else {
            return false;
        };
        selected.melded().contains(top) && Self::deadwood_after_discard(with_top) < deadwood(hand)
    }

    /// Reproduce `getUnmeldedCards`, with deterministic EAAI card ordering.
    #[must_use]
    pub(crate) fn discard_candidates(hand: Hand, taken: Option<Card>) -> Vec<(Card, u8)> {
        let partitions = all_minimum_melds(hand);
        let mut candidates: Vec<Card> = ordered_cards(hand)
            .into_iter()
            .filter(|&card| Some(card) != taken)
            .filter(|&card| {
                partitions
                    .iter()
                    .any(|partition| partition.deadwood_cards().contains(card))
            })
            .collect();

        let largest = candidates
            .iter()
            .map(|card| card.rank.deadwood())
            .max()
            .unwrap_or(0);
        candidates
            .retain(|card| card.rank.deadwood().saturating_add(MAX_DEADWOOD_DIFFERENCE) >= largest);

        // The Java player returns before calculating candidate statistics in
        // this case.  We calculate the one residual value only because the
        // combined host action must decide whether to knock.
        if candidates.len() == 1 {
            let card = candidates[0];
            return vec![(card, deadwood(hand - card.into()))];
        }

        // A fully melded eleven-card hand, or one whose sole loose card was
        // just taken, falls back to every legal discard.
        if candidates.is_empty() {
            candidates = ordered_cards(hand)
                .into_iter()
                .filter(|&card| Some(card) != taken)
                .collect();
        }

        let mut evaluated = Vec::with_capacity(candidates.len());
        let mut best_deadwood = u8::MAX;
        for card in candidates {
            let rest = deadwood(hand - card.into());
            // Source v5 clears the list at the first gin candidate rather
            // than comparing danger or collecting other gin discards.
            if rest == 0 {
                return vec![(card, rest)];
            }
            best_deadwood = best_deadwood.min(rest);
            evaluated.push((card, rest));
        }
        evaluated.retain(|&(_, rest)| {
            u16::from(rest) <= u16::from(best_deadwood) + u16::from(MAX_DEADWOOD_DIFFERENCE)
        });
        evaluated
    }

    fn future_deadwood_mean(post_discard: Hand, seen: Hand) -> f64 {
        let seen_bits = eaai_bits(seen);
        let mut outcomes = Vec::new();
        for id in 0..52 {
            if seen_bits & (1_u64 << id) != 0 {
                continue;
            }
            let next = post_discard | super::melds::card_from_eaai_id(id).into();
            outcomes.push(Self::deadwood_after_discard(next));
        }
        outcomes.sort_unstable();
        let count = outcomes.len().min(MAX_CARDS_TO_CONSIDER);
        if count == 0 {
            0.0
        } else {
            let total: u16 = outcomes[..count].iter().copied().map(u16::from).sum();
            f64::from(total) / count as f64
        }
    }

    /// MARJJ's exact opponent-help adjustment, including its source bugs.
    #[must_use]
    pub(crate) fn danger_score(card: Card, my_hand: Hand, seen: Hand, opponent_cards: Hand) -> f64 {
        let id = eaai_id(card);
        let rank = card.rank.get() - 1;
        let my_cards = eaai_bits(my_hand);
        let seen = eaai_bits(seen);
        let opponent_cards = eaai_bits(opponent_cards);

        let classify = |other_id: u8| {
            let bit = 1_u64 << other_id;
            (
                opponent_cards & bit != 0,
                opponent_cards & bit == 0 && (my_cards | seen) & bit != 0,
            )
        };

        let (opp_higher, mut unavailable_higher) = if rank < 12 {
            classify(id + 1)
        } else {
            (false, true)
        };
        let (opp_lower, unavailable_lower) = if rank > 0 {
            classify(id - 1)
        } else {
            (false, true)
        };
        let (opp_two_higher, unavailable_two_higher) = if rank < 11 {
            classify(id + 2)
        } else {
            // Intentional v5 typo: the Q/K boundary marks the one-away
            // higher card unavailable and leaves the two-away flag false.
            unavailable_higher = true;
            (false, false)
        };
        let (opp_two_lower, unavailable_two_lower) = if rank > 1 {
            classify(id - 2)
        } else {
            (false, true)
        };

        let mut opponent_same_rank = 0;
        let mut unavailable_same_rank = 0;
        for suit in 0..4 {
            let bit = 1_u64 << (rank + suit * 13);
            if opponent_cards & bit != 0 {
                opponent_same_rank += 1;
            } else if (my_cards | seen) & bit != 0 {
                unavailable_same_rank += 1;
            }
        }

        let mut score = 0.0;
        if (unavailable_higher && (unavailable_lower || unavailable_two_lower))
            || (unavailable_lower && (unavailable_higher || unavailable_two_higher))
        {
            score += SEQUENCE_BONUS;
        } else {
            if unavailable_lower {
                score += ADJ_BONUS;
            }
            if unavailable_higher {
                score += ADJ_BONUS;
            }
            if unavailable_two_lower {
                score += ADJ2_BONUS;
            }
            if unavailable_two_higher {
                score += ADJ2_BONUS;
            }
        }

        if (opp_higher && (opp_lower || opp_two_lower))
            || (opp_lower && (opp_higher || opp_two_higher))
        {
            score += SEQUENCE_PENALTY;
        } else {
            if opp_lower {
                score += ADJ_PENALTY;
            }
            if opp_higher {
                score += ADJ_PENALTY;
            }
            if opp_two_lower {
                score += ADJ2_PENALTY;
            }
            if opp_two_higher {
                score += ADJ2_PENALTY;
            }
        }

        if unavailable_same_rank >= 3 {
            score += SET_BONUS;
        } else if unavailable_same_rank == 2 {
            score += RANK_BONUS;
        }
        if opponent_same_rank >= 2 {
            score += SET_PENALTY;
        } else if unavailable_same_rank == 1 {
            // Intentional v5 bug: this condition checks the unavailable
            // count, not `opponent_same_rank == 1`.
            score += RANK_PENALTY;
        }
        score
    }

    /// Pure diagnostic form of the candidate evaluator used by conformance
    /// tests.  `turns` is the one-based count after entering `getDiscard`.
    #[must_use]
    pub(crate) fn evaluate_discards(
        hand: Hand,
        taken: Option<Card>,
        seen: Hand,
        opponent_cards: Hand,
        turns: u32,
    ) -> Vec<DiscardEvaluation> {
        assert!(turns > 0, "MARJJ increments its turn before evaluation");
        let decay = DECAY.powi(i32::try_from(turns - 1).unwrap_or(i32::MAX));
        Self::discard_candidates(hand, taken)
            .into_iter()
            .map(|(card, post_deadwood)| {
                let future_mean = Self::future_deadwood_mean(hand - card.into(), seen);
                let danger = Self::danger_score(card, hand, seen, opponent_cards);
                let total = f64::from(post_deadwood) + INIT_WEIGHT * future_mean * decay + danger;
                DiscardEvaluation {
                    card,
                    post_deadwood,
                    future_mean,
                    danger,
                    total,
                }
            })
            .collect()
    }

    /// Select a discard from an explicit legacy state.  This is the minimal
    /// RNG-bearing diagnostic used to verify exact-tie replay behavior.
    pub(crate) fn choose_discard_for_state(
        &mut self,
        hand: Hand,
        taken: Option<Card>,
        seen: Hand,
        opponent_cards: Hand,
        turns: u32,
    ) -> (Card, u8) {
        let candidates = Self::discard_candidates(hand, taken);
        if candidates.len() == 1 {
            return candidates[0];
        }

        let evaluations = Self::evaluate_discards(hand, taken, seen, opponent_cards, turns);
        let mut best_score = f64::INFINITY;
        let mut tied = Vec::new();
        for evaluation in evaluations {
            if evaluation.total < best_score {
                best_score = evaluation.total;
                tied.clear();
            }
            if evaluation.total == best_score {
                tied.push((evaluation.card, evaluation.post_deadwood));
            }
        }
        let index = self.rng.random_range(0..tied.len());
        tied[index]
    }

    /// Estimate how many points an opponent could lay off on one spread.
    /// This deliberately retains v5's endpoint-value off-by-one and its
    /// one-directional C -> H -> S -> D -> C set cycle.
    fn estimated_layoff(spread: Melds, seen: Hand, opponent_cards: Hand) -> u16 {
        let seen = eaai_bits(seen);
        let opponent = eaai_bits(opponent_cards);
        let available = |bit: u64| opponent & bit != 0 || seen & bit == 0;
        let mut points = 0;

        for meld in spread.iter() {
            match meld.kind() {
                MeldKind::Run => {
                    let low = meld.low().expect("a run has a low rank").get() - 1;
                    let high = meld.high().expect("a run has a high rank").get() - 1;
                    let first = ordered_cards(meld.cards())[0];
                    let suit_start = eaai_id(first) - low;
                    if low > 0 && available(1_u64 << (suit_start + low - 1)) {
                        points += u16::from((low + 1).min(10));
                    }
                    if high < 12 && available(1_u64 << (suit_start + high + 1)) {
                        points += u16::from((high + 1).min(10));
                    }
                }
                MeldKind::Set => {
                    for card in ordered_cards(meld.cards()) {
                        let next_suit = (eaai_id(card) + 13) % 52;
                        if available(1_u64 << next_suit) {
                            points += u16::from(card.rank.deadwood());
                        }
                    }
                }
            }
        }
        points
    }

    fn select_spread(hand: Hand, seen: Hand, opponent_cards: Hand) -> Option<Melds> {
        // The partition list is canonical, and Iterator::min_by_key keeps the
        // first item on a tie.  Thus v5's strict layoff-score comparison gets
        // a reproducible secondary tie-break.
        all_minimum_melds(hand)
            .into_iter()
            .min_by_key(|&spread| Self::estimated_layoff(spread, seen, opponent_cards))
    }

    fn knock_spread(&self, hand: Hand) -> Melds {
        Self::select_spread(hand, self.seen, self.opponent_cards)
            .unwrap_or_else(|| best_melds(hand))
    }

    /// The source asks for a voluntary knock only during completed turns
    /// one through three; gin remains mandatory thereafter.
    #[must_use]
    pub(crate) const fn should_knock(turns: u32, deadwood: u8, knock_limit: u8) -> bool {
        deadwood == 0 || (turns < 4 && deadwood <= 10 && deadwood <= knock_limit)
    }
}

impl<R: Rng> Strategy for MarjjV5Surrogate<R> {
    fn offer_upcard(&mut self, view: &View<'_>) -> UpcardAction {
        // Every player receives a fresh-game callback in the Java host.  The
        // first upcard offer is the corresponding reliable reset point here.
        self.reset(view);
        let top = self.observe_offered_card(view);
        if Self::would_take(view.hand(), top, self.seen, self.opponent_cards) {
            UpcardAction::Take
        } else {
            UpcardAction::Pass
        }
    }

    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction {
        // If the non-dealer took the opening upcard, the dealer never receives
        // `offer_upcard`.  A full stock plus an unexpected ten-card hand is
        // the host-side fallback for Java's `startGame` callback.
        if !self.initialized
            || (view.stock_len() == 31 && self.last_hand.is_some_and(|hand| hand != view.hand()))
        {
            self.reset(view);
        } else {
            self.observe(view);
        }
        let top = self.observe_offered_card(view);
        if Self::would_take(view.hand(), top, self.seen, self.opponent_cards) {
            DrawAction::TakeDiscard
        } else {
            DrawAction::Stock
        }
    }

    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        self.observe(view);
        self.turns += 1;
        let hand = view.hand();
        let (discard, rest) = self.choose_discard_for_state(
            hand,
            view.taken_discard(),
            self.seen,
            self.opponent_cards,
            self.turns,
        );
        let remaining = hand - discard.into();
        self.last_hand = Some(remaining);

        // MARJJ voluntarily knocks only on its first three completed turns;
        // after that it waits for gin.  Capping the early threshold by the
        // host's rule is the minimal adaptation needed to keep actions legal
        // outside the EAAI challenge's fixed ten-point rules.
        if Self::should_knock(self.turns, rest, view.knock_limit()) {
            TurnAction::Knock {
                discard,
                melds: self.knock_spread(remaining),
            }
        } else {
            TurnAction::Discard(discard)
        }
    }

    fn choose_layoff(&mut self, view: &View<'_>) -> Option<Layoff> {
        self.observe(view);
        self.last_hand = Some(view.hand());
        HeuristicBot::new().choose_layoff(view)
    }

    fn name(&self) -> &str {
        "marjj-v5-surrogate"
    }
}
