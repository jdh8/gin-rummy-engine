//! [`HeuristicBot`]: a deterministic knowledge-based player
//!
//! The knowledge-free greedy core in this module — draw only on strict
//! deadwood improvement, shed the least useful card, knock as soon as
//! allowed — is the policy the sibling crate's `simulate` example proved
//! out, and it doubles as the rollout policy of the Monte Carlo bot.  The
//! bot itself layers opponent knowledge on top: discards are penalized by
//! how much they could help the opponent's melds.

use crate::{DrawAction, Layoff, Strategy, TurnAction, UpcardAction, View};
use gin_rummy::{Card, Hand, Meld, Melds, Rank, Suit, best_melds, deadwood};

/// The discard leaving the least deadwood, skipping the just-taken card
///
/// Ties prefer shedding higher pip values, then the lowest card in hand
/// order.  This is the knowledge-free greedy shed shared with the Monte
/// Carlo rollout policy, where it is the hottest function of all: it runs
/// for both seats on every turn of every sampled world.  The caller
/// passes the hand's solved arrangement so that one solve also answers
/// its Big Gin check; see [`shed_by`] for how the candidates are priced
/// from it.
#[cfg(feature = "rand")]
pub(crate) fn best_shed(melds: Melds, taken: Option<Card>) -> (Card, u8) {
    shed_by(melds, taken, |card, rest| {
        (rest, u8::MAX - card.rank.deadwood())
    })
}

/// The discard minimizing `key(card, residual deadwood)`, skipping the
/// just-taken card, with ties going to the earlier card in hand order
///
/// Removing a card the arrangement leaves unmelded lowers the deadwood by
/// exactly that card's pips: the arrangement minus the card arranges the
/// smaller hand, and a better one would, with the card put back as
/// deadwood, beat the optimum on the full hand.  Removing a melded card
/// can do no better than that same bound.  So the unmelded cards are
/// priced without a solve, and a melded card gets one only when its bound
/// could still win — which is why `key` must not decrease as the residual
/// grows.  The result is exactly what solving every candidate returns,
/// tie-breaks included, at a fraction of the solves.
fn shed_by<K: Ord>(melds: Melds, taken: Option<Card>, key: impl Fn(Card, u8) -> K) -> (Card, u8) {
    let hand = melds.hand();
    let total = melds.deadwood();
    let unmelded = melds.deadwood_cards();
    // The hand position is the final tie-break, so the minimum of
    // `(key, position)` is the first card `min_by_key` would return.
    let mut best: Option<((K, usize), Card, u8)> = None;
    for (position, card) in hand.iter().enumerate() {
        if Some(card) == taken || !unmelded.contains(card) {
            continue;
        }
        let rest = total - card.rank.deadwood();
        let candidate = (key(card, rest), position);
        if best.as_ref().is_none_or(|(held, ..)| candidate < *held) {
            best = Some((candidate, card, rest));
        }
    }
    for (position, card) in hand.iter().enumerate() {
        if Some(card) == taken || unmelded.contains(card) {
            continue;
        }
        let bound = total.saturating_sub(card.rank.deadwood());
        if best
            .as_ref()
            .is_some_and(|(held, ..)| (key(card, bound), position) >= *held)
        {
            continue;
        }
        let rest = deadwood(hand - card.into());
        let candidate = (key(card, rest), position);
        if best.as_ref().is_none_or(|(held, ..)| candidate < *held) {
            best = Some((candidate, card, rest));
        }
    }
    best.map(|(_, card, rest)| (card, rest))
        .expect("a hand with a draw always has a legal discard")
}

/// Whether taking `top` strictly lowers deadwood after the best legal shed
/// (which may not be `top` itself)
///
/// `hand` is the ten-card hand before the draw and `top` lies outside it.
/// Only the existence of an improving shed matters, so this looks for any
/// candidate under the mark instead of ranking them, priced by the same
/// bound as [`shed_by`].
pub(crate) fn improves(hand: Hand, top: Card) -> bool {
    improving_melds(hand, top, None).is_some()
}

/// The solved draw when taking `top` strictly improves the hand.
///
/// Rollouts reuse the arrangement on their next shed and may supply the
/// ten-card deadwood from their previous shed to avoid solving it again.
/// When supplied, `before` must equal `deadwood(hand)`.
// Returning the arrangement out of line costs more than the saved solves
// in the decision benchmark; inlining lets the rollout retain it in place.
#[inline(always)]
pub(crate) fn improving_melds(hand: Hand, top: Card, before: Option<u8>) -> Option<Melds> {
    debug_assert!(!hand.contains(top), "the pile top is not in the hand");
    let with = hand | top.into();
    let melds = best_melds(with);
    let total = melds.deadwood();
    let unmelded = melds.deadwood_cards();
    // The bound prices the hand without `top` too when `top` is unmelded.
    let before = if unmelded.contains(top) {
        total - top.rank.deadwood()
    } else {
        before.unwrap_or_else(|| deadwood(hand))
    };
    let others = |cards: Hand| cards.iter().filter(move |&card| card != top);
    let improves = others(unmelded).any(|card| total - card.rank.deadwood() < before)
        || others(melds.melded()).any(|card| {
            total.saturating_sub(card.rank.deadwood()) < before
                && deadwood(with - card.into()) < before
        });
    improves.then_some(melds)
}

/// Whether `top` would sit inside some meld of `hand` + `top`
///
/// The EAAI-2021 baseline's test for drawing the face-up card, shared by
/// [`EaaiSimpleBot`](crate::EaaiSimpleBot) and the Monte Carlo rollout's
/// [`MeldOnly`](crate::OpponentModel::MeldOnly) opponent model, so the
/// modeled opponent and the actual baseline can never drift apart.
#[cfg(feature = "rand")]
pub(crate) fn joins_a_meld(hand: Hand, top: Card) -> bool {
    let with = hand | top.into();
    let of_rank = Suit::ASC
        .into_iter()
        .filter(|&suit| {
            with.contains(Card {
                suit,
                rank: top.rank,
            })
        })
        .count();
    if of_rank >= 3 {
        return true;
    }

    // Any three consecutive ranks of the card's suit around it; runs
    // never wrap, so the windows truncate at the ace and the king.
    let pivot = top.rank.get();
    (pivot.saturating_sub(2).max(1)..=pivot.min(11)).any(|low| {
        (low..low + 3).all(|rank| {
            with.contains(Card {
                suit: top.suit,
                rank: Rank::new(rank),
            })
        })
    })
}

/// The greedy layoff: the highest-pip own deadwood card that extends a
/// spread meld, with the target meld's index
///
/// Restricted to deadwood cards of an optimal arrangement — laying off a
/// melded card could *increase* final deadwood, since the defender's
/// remainder is melded optimally at settlement.  Shared with the Monte
/// Carlo rollout policy.
pub(crate) fn greedy_layoff(
    hand: Hand,
    spread: impl Iterator<Item = Meld>,
) -> Option<(Card, usize)> {
    let dead = best_melds(hand).deadwood_cards();
    spread
        .enumerate()
        .flat_map(|(index, meld)| {
            dead.iter()
                .filter(move |&card| meld.extended(card).is_some())
                .map(move |card| (card, index))
        })
        .max_by_key(|&(card, _)| card.rank.deadwood())
}

/// Tuning knobs for [`HeuristicBot`]
///
/// Like [`gin_rummy::Rules`], the struct is non-exhaustive: start from
/// [`HeuristicConfig::default`] and adjust fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct HeuristicConfig {
    /// Knock whenever the residual deadwood is at most
    /// `min(knock_limit, knock_threshold)`
    ///
    /// The default of 4 holds out past the first legal knock — banking a
    /// small knock every hand loses a game to the gin and undercut bonuses.
    /// Raise it toward the knock limit to knock as soon as the rules allow,
    /// or lower it to hunt gin.  [`score_awareness`](Self::score_awareness)
    /// bends this threshold by the game score at play time.
    pub knock_threshold: u8,
    /// Weight of discard safety against the opponent's revealed cards
    ///
    /// Zero ignores the opponent entirely, reproducing the pure greedy
    /// player.  The default is 1.
    pub safety_weight: u8,
    /// How strongly the game score shifts the knock threshold
    ///
    /// Points of threshold shift per unit of `margin / (game_target −
    /// leader_score)`, where `margin` is this seat's game-score lead and
    /// `leader_score` the higher of the two running totals.  Ahead the
    /// effective threshold rises toward the
    /// legal limit (bank the lead by knocking early); behind it falls
    /// toward zero (hold out for a gin that swings the deficit).  The
    /// denominator is the leader's distance to the winning line, not the
    /// full target, so the same lead bends the threshold ever harder as
    /// the game nears its end: a modest early-game nudge becomes a knock
    /// at any deadwood once the front-runner is a hand from winning.  Zero
    /// ignores the score, so a round played outside a game is unaffected.
    pub score_awareness: u8,
}

impl Default for HeuristicConfig {
    fn default() -> Self {
        // Tuned by whole-game self-play (see `examples/tune.rs`): holding
        // past the first legal knock and shifting the knock threshold by
        // the leader's distance to the winning line lift the heuristic's
        // game-win rate to ~50% against the Monte Carlo bot, up from ~42%
        // for score-blind play.
        Self {
            knock_threshold: 4,
            safety_weight: 1,
            score_awareness: 40,
        }
    }
}

/// A deterministic knowledge-based player
///
/// Fast enough for tournaments at any scale: every decision costs a few
/// deadwood-solver calls, each microseconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeuristicBot {
    config: HeuristicConfig,
}

impl HeuristicBot {
    /// A bot with the default configuration
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A bot with custom tuning
    #[must_use]
    pub const fn with_config(config: HeuristicConfig) -> Self {
        Self { config }
    }

    /// The cards that could meld with `card`: its rank in the other suits,
    /// and its suit within two ranks
    fn adjoiners(card: Card) -> Hand {
        let mut mask = Hand::EMPTY;
        for suit in Suit::ASC {
            if suit != card.suit {
                mask.insert(Card {
                    suit,
                    rank: card.rank,
                });
            }
        }
        let pivot = card.rank.get();
        for rank in pivot.saturating_sub(2).max(1)..=(pivot + 2).min(13) {
            if rank != pivot {
                mask.insert(Card {
                    suit: card.suit,
                    rank: Rank::new(rank),
                });
            }
        }
        mask
    }

    /// How much discarding `card` could help the opponent
    ///
    /// Adjoining cards the opponent is known to hold weigh double, unseen
    /// adjoiners weigh single, and adjoiners the opponent shed or declined
    /// count against — they signal disinterest, and shed adjoiners are
    /// physically unavailable.
    fn danger(view: &View<'_>, card: Card) -> i32 {
        let mask = Self::adjoiners(card);
        let known = (mask & view.opponent_known()).len() as i32;
        let unseen = (mask & view.unseen()).len() as i32;
        let cold = (mask & (view.opponent_shed() | view.opponent_passed())).len() as i32;
        2 * known + unseen - 2 * cold
    }

    /// The knock threshold in effect, shifted by the game score
    ///
    /// The neutral base is `knock_threshold`; `score_awareness` scales the
    /// shift by `(mine − theirs) / (game_target − leader_score)`.  Ahead the
    /// threshold rises (knock sooner), behind it falls toward zero (hold
    /// out for gin); dividing by the leader's distance to the line, not
    /// the full target, makes the same lead matter more late in the game.
    fn knock_threshold(&self, view: &View<'_>) -> u8 {
        let base = i32::from(self.config.knock_threshold);
        let [mine, theirs] = view.game_scores().map(i32::from);
        // The leader's distance to the winning line: the score bias grows
        // as the game nears its end, not merely with the raw margin.
        let remaining = i32::from(view.rules().game_target) - mine.max(theirs);
        let bias = i32::from(self.config.score_awareness) * (mine - theirs) / remaining.max(1);
        (base + bias).clamp(0, i32::from(u8::MAX)) as u8
    }

    /// The shed minimizing `(residual deadwood, weighted danger, -pips)`
    fn choose_shed(&self, view: &View<'_>, melds: Melds) -> (Card, u8) {
        let weight = i32::from(self.config.safety_weight);
        shed_by(melds, view.taken_discard(), |card, rest| {
            (
                rest,
                weight * Self::danger(view, card),
                u8::MAX - card.rank.deadwood(),
            )
        })
    }
}

impl Strategy for HeuristicBot {
    fn offer_upcard(&mut self, view: &View<'_>) -> UpcardAction {
        let top = view.upcard().expect("the upcard offer has an upcard");
        if improves(view.hand(), top) {
            UpcardAction::Take
        } else {
            UpcardAction::Pass
        }
    }

    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction {
        let top = view.upcard().expect("the pile is never empty on a draw");
        if improves(view.hand(), top) {
            DrawAction::TakeDiscard
        } else {
            DrawAction::Stock
        }
    }

    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        let hand = view.hand();
        let melds = best_melds(hand);
        if view.rules().big_gin_bonus.is_some() && melds.deadwood() == 0 {
            return TurnAction::BigGin(melds);
        }

        let (card, rest) = self.choose_shed(view, melds);
        if rest <= view.knock_limit().min(self.knock_threshold(view)) {
            TurnAction::Knock {
                discard: card,
                melds: best_melds(hand - card.into()),
            }
        } else {
            TurnAction::Discard(card)
        }
    }

    fn choose_layoff(&mut self, view: &View<'_>) -> Option<Layoff> {
        greedy_layoff(view.hand(), view.spread()).map(|(card, meld)| Layoff { card, meld })
    }

    fn name(&self) -> &str {
        "greedy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gin_rummy::Meld;
    use proptest::prelude::{Just, ProptestConfig, Strategy as _, prop_assert_eq, proptest};

    fn card(text: &str) -> Card {
        text.parse().expect("a valid card")
    }

    /// [`shed_by`] as it read before the single-solve pruning: one solver
    /// call per candidate.  The oracle the pruned version is held to.
    fn brute_shed<K: Ord>(
        hand: Hand,
        taken: Option<Card>,
        key: impl Fn(Card, u8) -> K,
    ) -> (Card, u8) {
        hand.iter()
            .filter(|&card| Some(card) != taken)
            .map(|card| (card, deadwood(hand - card.into())))
            .min_by_key(|&(card, rest)| key(card, rest))
            .expect("a hand with a draw always has a legal discard")
    }

    /// The rollout's key: least deadwood, then the highest pips
    fn greedy_key(card: Card, rest: u8) -> (u8, u8) {
        (rest, u8::MAX - card.rank.deadwood())
    }

    /// A key with a card-dependent middle term standing in for the
    /// heuristic's danger weight, so the pruning is tested with a
    /// tie-break that hand order alone does not settle.
    fn salted_key(card: Card, rest: u8) -> (u8, u8, u8) {
        (
            rest,
            card.rank.get().wrapping_mul(37) % 5,
            u8::MAX - card.rank.deadwood(),
        )
    }

    /// Shuffled decks of varying meld density: the full deck, two suits
    /// (no sets are possible, so runs carry every meld), and the low ranks
    /// (melds and equal-pip ties both common), so melded high cards and
    /// ties between melded and unmelded cards all come up often.
    fn dense_deck() -> impl proptest::strategy::Strategy<Value = Vec<Card>> {
        let decks = [
            Hand::ALL,
            Hand::ALL
                .iter()
                .filter(|card| matches!(card.suit, Suit::Hearts | Suit::Spades))
                .collect(),
            Hand::ALL
                .iter()
                .filter(|card| card.rank.get() <= 7)
                .collect(),
        ];
        (0..decks.len()).prop_flat_map(move |index| {
            Just(decks[index].iter().collect::<Vec<Card>>()).prop_shuffle()
        })
    }

    #[test]
    fn pruned_shed_matches_solving_every_candidate() {
        proptest!(
            ProptestConfig::with_cases(4000),
            |(deck in dense_deck(), taken in 0..12_usize)| {
                let hand: Hand = deck[..11].iter().copied().collect();
                // Index 11 skips nothing, the stock-draw case.
                let taken = deck[..11].get(taken).copied();
                let melds = best_melds(hand);
                let expected = brute_shed(hand, taken, greedy_key);
                prop_assert_eq!(shed_by(melds, taken, greedy_key), expected);
                #[cfg(feature = "rand")]
                prop_assert_eq!(best_shed(melds, taken), expected);
                if melds.deadwood_cards().contains(expected.0) {
                    // Reusing a knock spread must preserve the solver's
                    // exact tie-break and meld order, not just its score.
                    prop_assert_eq!(
                        melds.iter().collect::<Vec<_>>(),
                        best_melds(hand - expected.0.into()).iter().collect::<Vec<_>>()
                    );
                }
                prop_assert_eq!(
                    shed_by(melds, taken, salted_key),
                    brute_shed(hand, taken, salted_key)
                );

                let ten: Hand = deck[..10].iter().copied().collect();
                let top = deck[10];
                let (_, rest) = brute_shed(ten | top.into(), Some(top), greedy_key);
                prop_assert_eq!(improves(ten, top), rest < deadwood(ten));
                prop_assert_eq!(
                    improving_melds(ten, top, Some(deadwood(ten))),
                    (rest < deadwood(ten)).then(|| best_melds(ten | top.into()))
                );
            }
        );
    }

    #[cfg(feature = "rand")]
    #[test]
    fn best_shed_minimizes_deadwood_then_dumps_pips() {
        // ♣A♣2♣3 ♦4♦5♦6 ♥7♥8♥9 + ♠K ♠5: shedding the king keeps 5 deadwood.
        let hand: Hand = "A23.456.789.5K".parse().expect("a valid hand");
        assert_eq!(best_shed(best_melds(hand), None), (card("♠K"), 5));
        // The king may not be shed if it was just taken; the five goes.
        assert_eq!(
            best_shed(best_melds(hand), Some(card("♠K"))),
            (card("♠5"), 10)
        );
    }

    #[test]
    fn improves_is_strict() {
        let hand: Hand = "A2.456.789.5K".parse().expect("a valid hand");
        // The ♣3 completes A-2-3: taking it sheds the king, 3+5=8 < 18.
        assert!(improves(hand, card("♣3")));
        // The ♦T helps nothing over drawing blind.
        assert!(!improves(hand, card("♦T")));
    }

    #[cfg(feature = "rand")]
    #[test]
    fn joins_a_meld_draws_the_upcard_only_into_a_meld() {
        // ♣A♣2♣7 ♦7 ♥3♥4 ♠8♠9.
        let hand: Hand = "A27.7.34.89".parse().expect("a valid hand");
        // A third seven completes a set; the ♥5 extends 3-4 into a run.
        assert!(joins_a_meld(hand, card("♠7")));
        assert!(joins_a_meld(hand, card("♥5")));
        assert!(joins_a_meld(hand, card("♥2")));
        // The ♦3 pairs the ♥3 and neighbors the ♦7's suit but melds with
        // neither; the ♦K is loose entirely.
        assert!(!joins_a_meld(hand, card("♦3")));
        assert!(!joins_a_meld(hand, card("♦K")));
        // Rank edges truncate rather than wrap.
        assert!(joins_a_meld("QK...".parse().unwrap(), card("♣J")));
        assert!(!joins_a_meld("2K...".parse().unwrap(), card("♣A")));
    }

    #[test]
    fn adjoiners_cover_sets_and_run_neighbors() {
        let mask = HeuristicBot::adjoiners(card("♦7"));
        for adjoining in ["♣7", "♥7", "♠7", "♦5", "♦6", "♦8", "♦9"] {
            assert!(mask.contains(card(adjoining)), "{adjoining} adjoins ♦7");
        }
        assert_eq!(mask.len(), 7);
        // Edges truncate: nothing below the ace.
        assert_eq!(HeuristicBot::adjoiners(card("♣A")).len(), 5);
    }

    #[test]
    fn greedy_layoff_extends_runs_but_never_breaks_melds() {
        let spread = [
            Meld::run(Suit::Clubs, Rank::new(5), Rank::new(7)),
            Meld::set(Rank::new(9), Some(Suit::Spades)),
        ];
        // ♣8 extends the run.  The ♠9 would complete the nine-set, but it
        // is melded into the defender's own ♠9-T-J-Q run and never offered.
        let hand: Hand = "8...9TJQ".parse().expect("a valid hand");
        assert_eq!(
            greedy_layoff(hand, spread.iter().copied()),
            Some((card("♣8"), 0)),
        );

        // A card inside the defender's own meld is not offered: laying
        // off the ♥T would break T-J-Q into pure deadwood.
        let melded: Hand = "..TJQ.".parse().expect("a valid hand");
        let sets = [Meld::set(Rank::T, Some(Suit::Hearts))];
        assert_eq!(greedy_layoff(melded, sets.iter().copied()), None);
    }

    #[test]
    fn chained_layoffs_terminate() {
        let mut spread = [Meld::run(Suit::Clubs, Rank::new(5), Rank::new(7))];
        // ♣8 ♣9 are two loose cards: each extends the run once the other
        // has stretched it.
        let mut hand: Hand = "89...".parse().expect("a valid hand");
        let mut laid = Vec::new();
        while let Some((card, index)) = greedy_layoff(hand, spread.iter().copied()) {
            spread[index] = spread[index].extended(card).expect("a legal extension");
            hand.remove(card);
            laid.push(card);
        }
        assert_eq!(laid, ["♣8", "♣9"].map(card));
        assert!(hand.is_empty());
    }

    #[test]
    fn score_awareness_shifts_the_knock_threshold() {
        use crate::Table;
        use gin_rummy::{Player, Round, Rules};

        let deck: Vec<Card> = Hand::ALL.iter().collect();
        let hands = [
            deck.iter().step_by(2).take(10).copied().collect::<Hand>(),
            deck.iter().skip(1).step_by(2).take(10).copied().collect(),
        ];
        let round = Round::from_deal(
            Rules::default(),
            Player::One,
            hands,
            deck[20],
            deck[21..].to_vec(),
        )
        .expect("a partitioned deck");

        // Base 6, target 100: with a 60-point margin and the leader 40
        // points from the line the shift is 32 * 60 / 40 = 48, clamped
        // into knock-limit range.
        let bot = HeuristicBot::with_config(HeuristicConfig {
            knock_threshold: 6,
            score_awareness: 32,
            ..HeuristicConfig::default()
        });

        let ahead = Table::new(round.clone()).scores([60, 0]);
        let level = Table::new(round.clone());
        let behind = Table::new(round.clone()).scores([0, 60]);

        // Level score leaves the base untouched; ahead raises it, behind
        // drops it toward zero (hold out for gin).
        assert_eq!(bot.knock_threshold(&level.view(Player::One)), 6);
        assert!(
            bot.knock_threshold(&ahead.view(Player::One))
                > bot.knock_threshold(&level.view(Player::One))
        );
        assert!(bot.knock_threshold(&behind.view(Player::One)) < 6);

        // Proximity to the winning line, not the raw margin, drives the
        // shift: the same 10-point lead bends the threshold far more when
        // the leader is a hand from the target (denominator 10) than early
        // in the game (denominator 90).  The old target-normalized formula
        // scored these two equal.
        let near_line = Table::new(round.clone()).scores([90, 80]);
        let early = Table::new(round).scores([10, 0]);
        assert!(
            bot.knock_threshold(&near_line.view(Player::One))
                > bot.knock_threshold(&early.view(Player::One))
        );

        // A score-blind bot ignores the margin entirely.
        let blind = HeuristicBot::with_config(HeuristicConfig {
            knock_threshold: 6,
            score_awareness: 0,
            ..HeuristicConfig::default()
        });
        assert_eq!(blind.knock_threshold(&ahead.view(Player::One)), 6);
        assert_eq!(blind.knock_threshold(&behind.view(Player::One)), 6);
    }
}
