//! [`EaaiSimpleBot`]: the reference baseline of the EAAI-2021 challenge
//!
//! A port of Todd Neller's `SimpleGinRummyPlayer`, the opponent every
//! entry of the EAAI-2021 Gin Rummy Undergraduate Research Challenge was
//! measured against, so win rates against it are comparable across
//! engines and papers.  Its policy: draw the face-up card only if it
//! immediately joins a meld, shed a uniformly random card among those
//! leaving minimal deadwood, and knock as early as possible.

use crate::heuristic::greedy_layoff;
use crate::{DrawAction, Layoff, Strategy, TurnAction, UpcardAction, View};
use gin_rummy::{Card, Hand, Rank, Suit, best_melds, deadwood};
use rand::{Rng, RngExt as _};

/// Whether `top` would sit inside some meld of `hand` + `top` — the
/// baseline's test for drawing the face-up card.
fn joins_a_meld(hand: Hand, top: Card) -> bool {
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

/// The reference baseline of the EAAI-2021 Gin Rummy challenge
///
/// A port of `SimpleGinRummyPlayer` from Todd Neller's challenge
/// framework: it ignores everything the opponent does, takes the face-up
/// card only when it immediately joins a meld, sheds a uniformly random
/// card among those leaving minimal deadwood — refusing to repeat a
/// (draw, discard) pair within a round, the original's loop breaker —
/// and knocks at the first legal opportunity.  It exists as a fixed
/// measuring stick for comparisons across engines and papers, not as a
/// good player, so there are no tuning knobs.
///
/// Departures from the Java original, none of which change its policy
/// under the challenge rules: the knock spread is [`best_melds`]'s
/// optimal arrangement rather than a random optimal one (this affects
/// only which layoffs the defender is offered); layoffs use this crate's
/// greedy layoff in place of the framework's automatic first-fit sweep;
/// and the knock threshold follows [`View::knock_limit`] rather than a
/// hardcoded 10, so the bot stays legal under any ruleset.
///
/// The EAAI framework has no big gin, so this bot never declares it; run
/// benchmarks with `big_gin_bonus: None` for exactly the challenge's
/// round conditions.
#[derive(Debug, Clone)]
pub struct EaaiSimpleBot<R> {
    rng: R,
    /// The 10-card hand seen at this turn's draw decision, so `play_turn`
    /// can identify the drawn card even after a stock draw.
    pre_draw: Hand,
    /// (drawn, discarded) pairs already played this round; the original
    /// refuses to repeat one so mirror matches cannot loop forever.
    seen_pairs: Vec<(Card, Card)>,
}

impl<R: Rng> EaaiSimpleBot<R> {
    /// A baseline over the given generator; seed it for replayable runs
    #[must_use]
    pub const fn new(rng: R) -> Self {
        Self {
            rng,
            pre_draw: Hand::EMPTY,
            seen_pairs: Vec::new(),
        }
    }

    /// The card drawn this turn: the one added since the draw decision.
    fn drawn(&self, hand: Hand) -> Option<Card> {
        let fresh = hand - self.pre_draw;
        let mut cards = fresh.iter();
        match (cards.next(), cards.next()) {
            (Some(card), None) => Some(card),
            _ => None,
        }
    }

    /// The original's discard: uniformly random among the sheds leaving
    /// minimal deadwood, never the just-taken upcard, never a repeated
    /// (drawn, discarded) pair.  Records the chosen pair.
    fn pick_discard(&mut self, hand: Hand, taken: Option<Card>, drawn: Option<Card>) -> (Card, u8) {
        let mut min_deadwood = u8::MAX;
        let mut candidates: Vec<Card> = Vec::new();
        for card in hand {
            if Some(card) == taken {
                continue;
            }
            if let Some(drawn) = drawn
                && self.seen_pairs.contains(&(drawn, card))
            {
                continue;
            }
            let rest = deadwood(hand - card.into());
            if rest < min_deadwood {
                min_deadwood = rest;
                candidates.clear();
            }
            if rest <= min_deadwood {
                candidates.push(card);
            }
        }

        // The original would crash with every card blocked; forgetting
        // the pair memory instead keeps the bot legal.
        let (card, rest) = if candidates.is_empty() {
            self.seen_pairs.clear();
            hand.iter()
                .filter(|&card| Some(card) != taken)
                .map(|card| (card, deadwood(hand - card.into())))
                .min_by_key(|&(_, rest)| rest)
                .expect("an 11-card hand always has a legal discard")
        } else {
            let index = self.rng.random_range(0..candidates.len());
            (candidates[index], min_deadwood)
        };
        if let Some(drawn) = drawn {
            self.seen_pairs.push((drawn, card));
        }
        (card, rest)
    }
}

impl<R: Rng> Strategy for EaaiSimpleBot<R> {
    fn offer_upcard(&mut self, view: &View<'_>) -> UpcardAction {
        // The upcard offer opens a round: forget the previous round.
        self.seen_pairs.clear();
        self.pre_draw = view.hand();
        let top = view.upcard().expect("the upcard offer has an upcard");
        if joins_a_meld(view.hand(), top) {
            UpcardAction::Take
        } else {
            UpcardAction::Pass
        }
    }

    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction {
        // A full stock means nothing has been drawn yet: this is the
        // dealer's first decision of a round whose upcard offer it never
        // received, so the pair memory is stale.  (A chain of pile takes
        // can re-trigger this a turn later and drop one recorded pair —
        // harmless.)
        if view.stock_len() == 31 {
            self.seen_pairs.clear();
        }
        self.pre_draw = view.hand();
        let top = view.upcard().expect("the pile is never empty on a draw");
        if joins_a_meld(view.hand(), top) {
            DrawAction::TakeDiscard
        } else {
            DrawAction::Stock
        }
    }

    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        let hand = view.hand();
        let drawn = view.taken_discard().or_else(|| self.drawn(hand));
        let (card, rest) = self.pick_discard(hand, view.taken_discard(), drawn);

        // "Knock as early as possible."  The framework has no big gin, so
        // a fully melded 11-card hand knocks into plain gin instead.
        if rest <= view.knock_limit() {
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
        "eaai-simple"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HeuristicBot, play_game, play_round};
    use gin_rummy::{Game, Player, Round, Rules};
    use rand::SeedableRng as _;
    use rand::rngs::StdRng;

    fn card(text: &str) -> Card {
        text.parse().expect("a valid card")
    }

    #[test]
    fn draws_the_upcard_only_into_a_meld() {
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
    fn discards_uniformly_among_minimal_deadwood_sheds() {
        // ♣A♣2♣3 ♦4♦5♦6 ♥7♥8♥9 melded; the ♠J and drawn ♠K tie as sheds.
        let hand: Hand = "A23.456.789.JK".parse().expect("a valid hand");
        let mut bot = EaaiSimpleBot::new(StdRng::seed_from_u64(0));
        let mut seen = Hand::EMPTY;
        for _ in 0..64 {
            bot.seen_pairs.clear();
            let (shed, rest) = bot.pick_discard(hand, None, Some(card("♠K")));
            assert_eq!(rest, 10);
            seen.insert(shed);
        }
        // Both minimal sheds appear over 64 draws; melded cards never do.
        assert_eq!(seen, "...JK".parse().expect("a valid hand"));
    }

    #[test]
    fn refuses_a_repeated_draw_discard_pair() {
        let hand: Hand = "A23.456.789.JK".parse().expect("a valid hand");
        // Both minimal sheds are burned pairs: the next-least deadwood
        // shed breaks the club run at its cheapest card, uniquely ♣3.
        let mut bot = EaaiSimpleBot::new(StdRng::seed_from_u64(0));
        bot.seen_pairs.push((card("♠K"), card("♠J")));
        bot.seen_pairs.push((card("♠K"), card("♠K")));
        assert_eq!(
            bot.pick_discard(hand, None, Some(card("♠K"))),
            (card("♣3"), 23),
        );

        // With every card blocked the memory resets instead of panicking.
        let mut bot = EaaiSimpleBot::new(StdRng::seed_from_u64(0));
        for shed in hand {
            bot.seen_pairs.push((card("♠K"), shed));
        }
        let (_, rest) = bot.pick_discard(hand, None, Some(card("♠K")));
        assert_eq!(rest, 10);
        assert_eq!(bot.seen_pairs.len(), 1);
    }

    #[test]
    fn knocks_at_the_first_opportunity() {
        // Dealt three melds and the ♠J, One passes the useless ♠T, draws
        // the ♠K from the stock, and knocks on its very first turn.
        let hands: [Hand; 2] = [
            "A23.456.789.J".parse().expect("a valid hand"),
            "45.89J.JQK.23".parse().expect("a valid hand"),
        ];
        let upcard = card("♠T");
        let stock: Vec<Card> = Hand::ALL
            .iter()
            .filter(|&c| !hands[0].contains(c) && !hands[1].contains(c) && c != upcard)
            .collect();
        let round = Round::from_deal(Rules::default(), Player::Two, hands, upcard, stock)
            .expect("a partitioned deck");

        let mut bot = EaaiSimpleBot::new(StdRng::seed_from_u64(0));
        let mut greedy = HeuristicBot::new();
        let result = play_round(round, [&mut bot, &mut greedy]).expect("only legal actions");
        assert_eq!(result.winner(), Some(Player::One));
    }

    #[test]
    fn survives_whole_games_against_the_heuristic() {
        // The challenge rules have no big gin; whole games exercise the
        // per-round reset of the pair memory and every decision method.
        let mut rules = Rules::default();
        rules.big_gin_bonus = None;
        for seed in 0..8 {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut bot = EaaiSimpleBot::new(StdRng::seed_from_u64(seed));
            let mut greedy = HeuristicBot::new();
            let mut game = Game::new(rules, Player::One);
            play_game(&mut game, [&mut bot, &mut greedy], &mut rng).expect("only legal actions");
        }
    }
}
