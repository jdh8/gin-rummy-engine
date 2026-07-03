//! [`MonteCarloBot`]: determinized Monte Carlo move selection

use crate::heuristic::greedy_layoff;
use crate::sim::{Sim, SimPhase};
use crate::{DrawAction, Layoff, Strategy, TurnAction, UpcardAction, View};
use gin_rummy::deck::Deck;
use gin_rummy::{Card, Hand, Player, RoundResult, Rules, best_melds, deadwood};
use rand::Rng;

/// One determinized world: a concrete opponent hand and stock order
/// consistent with a [`View`]
struct World {
    opponent: Hand,
    /// Face-down draw order: the last element is drawn first
    stock: Vec<Card>,
}

/// A determinized Monte Carlo player
///
/// At every decision the bot samples hidden worlds consistent with its
/// [`View`] — the opponent holds every card they are known to have taken,
/// and the remaining unseen cards are distributed between their hand and
/// the stock, biased toward the meld-rich hands a real opponent collects —
/// plays each world out with the greedy policy on both seats, and picks
/// the action with the best expected score in round points.  The same
/// worlds are reused across candidate actions (common random numbers), and
/// the bot deviates from the greedy baseline only when the paired samples
/// show a statistically clear gain.
///
/// The bot owns its random number generator, so a seeded generator makes
/// its play reproducible.
pub struct MonteCarloBot<R: Rng> {
    rng: R,
    samples: u32,
    max_candidates: usize,
}

impl<R: Rng> MonteCarloBot<R> {
    /// A bot with default strength: 128 worlds per decision, 4 candidate
    /// discards
    pub const fn new(rng: R) -> Self {
        Self {
            rng,
            samples: 128,
            max_candidates: 4,
        }
    }

    /// Set how many worlds each decision samples
    ///
    /// More samples play stronger and slower.  At the default of 128 the
    /// bot wins about 56% of decisive rounds against [`HeuristicBot`]
    /// (~10 ms per turn in release builds); 32 keeps a smaller edge at a
    /// quarter of the cost.
    ///
    /// [`HeuristicBot`]: crate::HeuristicBot
    #[must_use]
    pub const fn samples(mut self, samples: u32) -> Self {
        self.samples = samples;
        self
    }

    /// Set how many candidate discards `play_turn` evaluates
    #[must_use]
    pub const fn max_candidates(mut self, max_candidates: usize) -> Self {
        self.max_candidates = max_candidates;
        self
    }

    /// Sample determinized worlds consistent with the view
    ///
    /// The opponent's hidden cards are not sampled uniformly: a real
    /// opponent has been collecting melds since the deal, so a uniform
    /// hand would be far too weak and the rollouts would recommend
    /// hunting gin against an opponent who never knocks.  Each world
    /// instead keeps the lowest-deadwood of several uniform draws, more
    /// of them the longer the round has run.
    fn sample_worlds(&mut self, view: &View<'_>) -> Vec<World> {
        let unseen = view.unseen();
        let known = view.opponent_known();
        let missing = view.opponent_hand_len() - known.len();
        // The pile grows by one card per turn played.
        let strength = (view.discard_pile().len() / 2).clamp(1, 6);

        (0..self.samples)
            .map(|_| {
                let hidden = (0..strength)
                    .map(|_| {
                        let mut pool = Deck::EMPTY;
                        for card in unseen {
                            pool.insert(card);
                        }
                        pool.draw(&mut self.rng, missing)
                    })
                    .min_by_key(|&hidden| deadwood(known | hidden))
                    .expect("at least one draw is always sampled");

                let mut pool = Deck::EMPTY;
                for card in unseen - hidden {
                    pool.insert(card);
                }
                let mut stock = Vec::with_capacity(pool.len());
                while let Some(card) = pool.pop(&mut self.rng) {
                    stock.push(card);
                }
                World {
                    opponent: known | hidden,
                    stock,
                }
            })
            .collect()
    }

    /// Per-world signed scores of `rollout` (common random numbers: every
    /// candidate sees the same worlds, so paired comparisons cancel most of
    /// the rollout noise)
    fn scores(
        view: &View<'_>,
        worlds: &[World],
        phase: SimPhase,
        rollout: impl Fn(Sim) -> RoundResult,
    ) -> Vec<f64> {
        let me = view.seat();
        let rules = view.rules();
        worlds
            .iter()
            .map(|world| score(rollout(Self::sim(view, world, phase)), me, rules))
            .collect()
    }

    /// Instantiate one world as a rollout state, to act at `phase`
    fn sim(view: &View<'_>, world: &World, phase: SimPhase) -> Sim {
        let seat = view.seat();
        let mut hands = [Hand::EMPTY; 2];
        hands[seat as usize] = view.hand();
        hands[seat.opponent() as usize] = world.opponent;
        Sim {
            rules: *view.rules(),
            knock_limit: view.knock_limit(),
            hands,
            stock: world.stock.clone(),
            pile: view.discard_pile().to_vec(),
            turn: seat,
            phase,
            taken: view.taken_discard(),
            // In the upcard phase, the dealer decides second.
            passes: u8::from(seat == view.dealer()),
            forced_stock: false,
        }
    }
}

/// Whether the challenger's paired advantage over the incumbent is large
/// enough to trust
///
/// The true value difference between most candidate actions is well below
/// the rollout noise floor, and deviating from the solid greedy baseline on
/// noise alone plays *worse* than the baseline.  A one-sided paired test —
/// the mean difference at least two standard errors above zero, since
/// several challengers get tested per decision — keeps only the deviations
/// the samples actually support.
fn beats(challenger: &[f64], incumbent: &[f64]) -> bool {
    let n = challenger.len() as f64;
    let mean = challenger
        .iter()
        .zip(incumbent)
        .map(|(c, i)| c - i)
        .sum::<f64>()
        / n;
    if mean <= 0.0 {
        return false;
    }
    let var = challenger
        .iter()
        .zip(incumbent)
        .map(|(c, i)| (c - i - mean).powi(2))
        .sum::<f64>()
        / n;
    mean > 2.0 * (var / n).sqrt()
}

/// The signed value of a result from `me`'s seat, in round points
fn score(result: RoundResult, me: Player, rules: &Rules) -> f64 {
    match result.winner() {
        Some(winner) if winner == me => f64::from(result.points(rules)),
        Some(_) => -f64::from(result.points(rules)),
        None => 0.0,
    }
}

impl<R: Rng> Strategy for MonteCarloBot<R> {
    fn offer_upcard(&mut self, view: &View<'_>) -> UpcardAction {
        let top = view.upcard().expect("the upcard offer has an upcard");
        let incumbent = if crate::heuristic::improves(view.hand(), top) {
            UpcardAction::Take
        } else {
            UpcardAction::Pass
        };

        let worlds = self.sample_worlds(view);
        let take = Self::scores(view, &worlds, SimPhase::Upcard, |mut sim| {
            sim.take_discard();
            sim.rollout()
        });
        let pass = Self::scores(view, &worlds, SimPhase::Upcard, |mut sim| {
            sim.pass();
            sim.rollout()
        });

        let (defend, challenge, challenger) = match incumbent {
            UpcardAction::Take => (take, pass, UpcardAction::Pass),
            UpcardAction::Pass => (pass, take, UpcardAction::Take),
        };
        if beats(&challenge, &defend) {
            challenger
        } else {
            incumbent
        }
    }

    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction {
        let top = view.upcard().expect("the pile is never empty on a draw");
        let incumbent = if crate::heuristic::improves(view.hand(), top) {
            DrawAction::TakeDiscard
        } else {
            DrawAction::Stock
        };

        let worlds = self.sample_worlds(view);
        let stock = Self::scores(view, &worlds, SimPhase::Draw, |mut sim| {
            sim.draw_stock();
            sim.rollout()
        });
        let pile = Self::scores(view, &worlds, SimPhase::Draw, |mut sim| {
            sim.take_discard();
            sim.rollout()
        });

        let (defend, challenge, challenger) = match incumbent {
            DrawAction::TakeDiscard => (pile, stock, DrawAction::Stock),
            DrawAction::Stock => (stock, pile, DrawAction::TakeDiscard),
        };
        if beats(&challenge, &defend) {
            challenger
        } else {
            incumbent
        }
    }

    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        let hand = view.hand();
        if deadwood(hand) == 0 && view.rules().big_gin_bonus.is_some() {
            // Big gin scores at least as much as gin under every ruleset.
            return TurnAction::BigGin(best_melds(hand));
        }

        // Rank legal sheds greedily and keep the most promising few; the
        // first candidate's knock-if-legal action is the greedy incumbent.
        let mut candidates: Vec<(Card, u8)> = hand
            .iter()
            .filter(|&card| Some(card) != view.taken_discard())
            .map(|card| (card, deadwood(hand - card.into())))
            .collect();
        candidates.sort_by_key(|&(card, rest)| (rest, u8::MAX - card.rank.deadwood()));
        candidates.truncate(self.max_candidates.max(1));

        let worlds = self.sample_worlds(view);
        let limit = view.knock_limit();
        let actions: Vec<(TurnAction, Vec<f64>)> = candidates
            .iter()
            .flat_map(|&(card, rest)| {
                let melds = best_melds(hand - card.into());
                let knock = (rest <= limit).then(|| {
                    let scores =
                        Self::scores(view, &worlds, SimPhase::Shed, |sim| sim.knock(card, melds));
                    (
                        TurnAction::Knock {
                            discard: card,
                            melds,
                        },
                        scores,
                    )
                });
                let discard = Self::scores(view, &worlds, SimPhase::Shed, |mut sim| {
                    sim.discard(card).unwrap_or_else(|| sim.rollout())
                });
                knock
                    .into_iter()
                    .chain(std::iter::once((TurnAction::Discard(card), discard)))
            })
            .collect();

        // Deviate from the incumbent only on statistically clear gains,
        // taking the largest such gain.
        let (incumbent, defend) = &actions[0];
        actions[1..]
            .iter()
            .filter(|(_, challenge)| beats(challenge, defend))
            .max_by(|(_, a), (_, b)| {
                let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
                mean(a).total_cmp(&mean(b))
            })
            .map_or(*incumbent, |(action, _)| *action)
    }

    fn choose_layoff(&mut self, view: &View<'_>) -> Option<Layoff> {
        // The round is over bar settlement; the greedy layoff is
        // near-exact and simulation adds nothing.
        greedy_layoff(view.hand(), view.spread()).map(|(card, meld)| Layoff { card, meld })
    }

    fn name(&self) -> &str {
        "mc"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Table;
    use gin_rummy::{Round, Rules};
    use rand::SeedableRng as _;
    use rand::rngs::StdRng;

    fn fixed_table() -> Table {
        let deck: Vec<_> = Hand::ALL.iter().collect();
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
        Table::new(round)
    }

    #[test]
    fn sampled_worlds_are_consistent_with_the_view() {
        let table = fixed_table();
        let view = table.view(Player::Two);
        let mut bot = MonteCarloBot::new(StdRng::seed_from_u64(1)).samples(32);

        for world in bot.sample_worlds(&view) {
            // Right sizes: a full opponent hand and the whole stock.
            assert_eq!(world.opponent.len(), view.opponent_hand_len());
            assert_eq!(world.stock.len(), view.stock_len());

            // Placement is a partition of the unseen cards...
            let stock: Hand = world.stock.iter().copied().collect();
            assert!((world.opponent & stock).is_empty());
            assert_eq!(
                world.opponent | stock,
                view.unseen() | view.opponent_known()
            );

            // ...that never touches what this seat can see.
            assert!((world.opponent & view.hand()).is_empty());
            assert!((stock & view.hand()).is_empty());
            assert_eq!(
                world.opponent & view.opponent_known(),
                view.opponent_known()
            );
        }
    }

    #[test]
    fn seeded_bots_repeat_their_decisions() {
        let table = fixed_table();
        let decide = |seed| {
            let mut bot = MonteCarloBot::new(StdRng::seed_from_u64(seed)).samples(16);
            bot.offer_upcard(&table.view(Player::Two))
        };
        assert_eq!(decide(3), decide(3));
    }

    #[test]
    fn beats_requires_a_clear_margin() {
        // A small mean edge buried in noise is not enough: the paired
        // differences swing ±1 around a +0.05 mean.
        let base: Vec<f64> = (0..32).map(|i| f64::from(i % 5)).collect();
        let noisy: Vec<f64> = base
            .iter()
            .enumerate()
            .map(|(i, x)| x + if i % 2 == 0 { 1.05 } else { -0.95 })
            .collect();
        assert!(!beats(&noisy, &base));

        // A consistent advantage is.
        let better: Vec<f64> = base.iter().map(|x| x + 1.0).collect();
        assert!(beats(&better, &base));
        assert!(!beats(&base, &better));
        // Equality never beats.
        assert!(!beats(&base, &base));
    }
}
