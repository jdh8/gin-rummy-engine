//! [`HeuristicBot`]: a deterministic knowledge-based player
//!
//! The knowledge-free greedy core in this module — draw only on strict
//! deadwood improvement, shed the least useful card, knock as soon as
//! allowed — is the policy the sibling crate's `simulate` example proved
//! out, and it doubles as the rollout policy of the Monte Carlo bot.  The
//! bot itself layers opponent knowledge on top: discards are penalized by
//! how much they could help the opponent's melds.

use crate::{DrawAction, Layoff, Strategy, TurnAction, UpcardAction, View};
use gin_rummy::{Card, Hand, Meld, Rank, Suit, best_melds, deadwood};

/// The discard leaving the least deadwood, skipping the just-taken card
///
/// Ties prefer shedding higher pip values, then the lowest card in hand
/// order.  This is the knowledge-free greedy shed shared with the Monte
/// Carlo rollout policy.
pub(crate) fn best_shed(hand: Hand, taken: Option<Card>) -> (Card, u8) {
    hand.iter()
        .filter(|&card| Some(card) != taken)
        .map(|card| (card, deadwood(hand - card.into())))
        .min_by_key(|&(card, rest)| (rest, u8::MAX - card.rank.deadwood()))
        .expect("a hand with a draw always has a legal discard")
}

/// Whether taking `top` strictly lowers deadwood after the best legal shed
/// (which may not be `top` itself)
pub(crate) fn improves(hand: Hand, top: Card) -> bool {
    let with = hand | top.into();
    let (_, rest) = best_shed(with, Some(top));
    rest < deadwood(hand)
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
    /// Points of threshold shift per unit of `game_margin / (game_target −
    /// leader_score)`, where `leader_score` is the higher of the two
    /// running totals.  Ahead the effective threshold rises toward the
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
    /// shift by `game_margin / (game_target − leader_score)`.  Ahead the
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
    fn choose_shed(&self, view: &View<'_>) -> (Card, u8) {
        let hand = view.hand();
        let taken = view.taken_discard();
        let weight = i32::from(self.config.safety_weight);
        hand.iter()
            .filter(|&card| Some(card) != taken)
            .map(|card| (card, deadwood(hand - card.into())))
            .min_by_key(|&(card, rest)| {
                (
                    rest,
                    weight * Self::danger(view, card),
                    u8::MAX - card.rank.deadwood(),
                )
            })
            .expect("an 11-card hand always has a legal discard")
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
        if deadwood(hand) == 0 && view.rules().big_gin_bonus.is_some() {
            return TurnAction::BigGin(best_melds(hand));
        }

        let (card, rest) = self.choose_shed(view);
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

    fn card(text: &str) -> Card {
        text.parse().expect("a valid card")
    }

    #[test]
    fn best_shed_minimizes_deadwood_then_dumps_pips() {
        // ♣A♣2♣3 ♦4♦5♦6 ♥7♥8♥9 + ♠K ♠5: shedding the king keeps 5 deadwood.
        let hand: Hand = "A23.456.789.5K".parse().expect("a valid hand");
        assert_eq!(best_shed(hand, None), (card("♠K"), 5));
        // The king may not be shed if it was just taken; the five goes.
        assert_eq!(best_shed(hand, Some(card("♠K"))), (card("♠5"), 10));
    }

    #[test]
    fn improves_is_strict() {
        let hand: Hand = "A2.456.789.5K".parse().expect("a valid hand");
        // The ♣3 completes A-2-3: taking it sheds the king, 3+5=8 < 18.
        assert!(improves(hand, card("♣3")));
        // The ♦T helps nothing over drawing blind.
        assert!(!improves(hand, card("♦T")));
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
