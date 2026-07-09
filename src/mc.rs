//! [`MonteCarloBot`]: determinized Monte Carlo move selection

use crate::heuristic::greedy_layoff;
use crate::sim::{Sim, SimPhase};
use crate::{DrawAction, Layoff, Strategy, TurnAction, UpcardAction, View};
use gin_rummy::deck::Deck;
use gin_rummy::{Card, Hand, Melds, Phase, Player, RoundResult, Rules, best_melds, deadwood};
use rand::Rng;

/// How many candidate discards `play_turn` and `assess` weigh at a discard:
/// the few lowest-deadwood sheds, the rest never worth a rollout.
const MAX_CANDIDATES: usize = 4;

/// One determinized world: a concrete opponent hand and stock order
/// consistent with a [`View`]
struct World {
    opponent: Hand,
    /// Face-down draw order: the last element is drawn first
    stock: Vec<Card>,
}

/// One candidate action to score, as owned data `assess` rolls through the
/// sampled worlds
struct Candidate {
    /// The rendered [`Assessment::action`] label.
    label: String,
    /// The rollout's starting phase, passed to [`MonteCarloBot::sim`].
    phase: SimPhase,
    /// The move to apply before the greedy playout.
    action: RolloutAction,
}

/// The move a [`Candidate`] applies to a [`Sim`] before rolling it out
///
/// Owned data, so `assess` can hold a list of candidates and roll each one
/// through every sampled world.
#[derive(Clone, Copy)]
enum RolloutAction {
    /// Take the pile — the upcard offer or a draw — then play on.
    TakeDiscard,
    /// Pass the upcard offer, then play on.
    Pass,
    /// Draw the hidden stock, then play on.
    DrawStock,
    /// Declare big gin: all eleven cards meld.
    BigGin,
    /// Knock, dropping `discard` with the rest laid as `melds`.
    Knock { discard: Card, melds: Melds },
    /// Shed `discard`, then play on (or settle if it ends the round).
    Discard(Card),
}

impl RolloutAction {
    /// Apply the move to a fresh rollout state and play it to a result.
    fn roll(self, mut sim: Sim) -> RoundResult {
        match self {
            Self::TakeDiscard => {
                sim.take_discard();
                sim.rollout()
            }
            Self::Pass => {
                sim.pass();
                sim.rollout()
            }
            Self::DrawStock => {
                sim.draw_stock();
                sim.rollout()
            }
            Self::BigGin => sim.big_gin(),
            Self::Knock { discard, melds } => sim.knock(discard, melds),
            Self::Discard(card) => sim.discard(card).unwrap_or_else(|| sim.rollout()),
        }
    }
}

/// A determinized Monte Carlo player
///
/// At every decision the bot samples hidden worlds consistent with its
/// [`View`] — the opponent holds every card they are known to have taken,
/// and the remaining unseen cards are distributed between their hand and
/// the stock, biased toward the meld-rich hands a real opponent collects —
/// plays each world out with the greedy policy on both seats, and picks
/// the action with the best expected value *for the game*: each rollout's
/// result lands on the running [`game scores`](View::game_scores), a
/// result that reaches [`game_target`](Rules::game_target) counts as the
/// win or loss of the game it is, and anything short of one counts its
/// round points.  The same worlds are reused across candidate actions
/// (common random numbers), and the bot deviates from the greedy baseline
/// only when the paired samples show a statistically clear gain.
///
/// The bot owns its random number generator, so a seeded generator makes
/// its play reproducible.
pub struct MonteCarloBot<R: Rng> {
    rng: R,
    samples: u32,
}

/// One candidate action's Monte Carlo assessment, for a solver or hint view
///
/// Produced by [`MonteCarloBot::assess`]: the same rollouts the bot chooses
/// with, surfaced per candidate instead of collapsed to the single action a
/// [`Strategy`] method returns.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Assessment {
    /// A rendered label for the action, e.g. `"discard 4♠"`, `"knock"`,
    /// `"take 4♠"`, `"pass"`, `"draw stock"`, `"big gin"`.
    pub action: String,
    /// Mean game-winning equity in `[0, 1]` — the quantity the bot
    /// maximizes, so candidates rank by it.
    pub equity: f64,
    /// Mean signed round points the action wins the deciding seat: positive
    /// for a net gain, negative for a net loss.
    pub ev: f64,
    /// Whether this is the bot's own pick — the move a [`Strategy`] method
    /// would return on this view.  Because the bot deviates from the greedy
    /// baseline only on a statistically clear gain, this need not be the
    /// highest-equity candidate.
    pub recommended: bool,
}

impl<R: Rng> MonteCarloBot<R> {
    /// A bot with default strength: 128 worlds per decision
    pub const fn new(rng: R) -> Self {
        Self { rng, samples: 128 }
    }

    /// Set how many worlds each decision samples
    ///
    /// More samples play stronger and slower.  At the default of 128 the
    /// bot wins about 65% of decisive rounds against the default
    /// [`HeuristicBot`] — which is tuned for whole-game play and so concedes
    /// single rounds — at ~10 ms per turn in release builds; 32 keeps a
    /// smaller edge at a quarter of the cost.
    ///
    /// [`HeuristicBot`]: crate::HeuristicBot
    #[must_use]
    pub const fn samples(mut self, samples: u32) -> Self {
        self.samples = samples;
        self
    }

    /// Sample determinized worlds consistent with the view
    ///
    /// The opponent's hidden cards are not sampled uniformly: a real
    /// opponent has been collecting melds since the deal, so a uniform
    /// hand would be far too weak and the rollouts would recommend
    /// hunting gin against an opponent who never knocks.  Each world
    /// instead keeps the lowest-deadwood of several uniform draws, more of
    /// them the deeper the pile — see [`opponent_strength`] — so the bias
    /// keeps intensifying for the whole round instead of leveling off
    /// partway through it.
    fn sample_worlds(&mut self, view: &View<'_>, count: u32) -> Vec<World> {
        let unseen = view.unseen();
        let known = view.opponent_known();
        let missing = view.opponent_hand_len() - known.len();
        let strength = opponent_strength(view.discard_pile().len());

        (0..count)
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

    /// Per-world game-winning equities of `rollout` (common random numbers:
    /// every candidate sees the same worlds, so paired comparisons cancel
    /// most of the rollout noise)
    fn equities(
        view: &View<'_>,
        worlds: &[World],
        phase: SimPhase,
        rollout: impl Fn(Sim) -> RoundResult,
    ) -> Vec<f64> {
        let me = view.seat();
        let rules = view.rules();
        let standing = view.game_scores();
        worlds
            .iter()
            .map(|world| equity(rollout(Self::sim(view, world, phase)), me, standing, rules))
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

    /// Assess every candidate action for the current decision, each with its
    /// Monte Carlo equity and expected round points, ranked by equity with
    /// the bot's own pick flagged — the read a solver or hint view shows
    ///
    /// The candidates and the flagged pick mirror the matching [`Strategy`]
    /// method on the same sampled worlds, so the recommended row is the move
    /// the bot would play — with one deliberate contraction: a knock's shed
    /// is not a real choice (dropping the largest deadwood is always the best
    /// knock), so the discard phase lists a single knock rather than one per
    /// shed.  Returns empty when the seat has no real choice: a forced stock
    /// draw, the layoff phase, or a finished round.
    #[must_use]
    pub fn assess(&mut self, view: &View<'_>) -> Vec<Assessment> {
        let candidates = self.hint_candidates(view);
        if candidates.is_empty() {
            return Vec::new();
        }
        let worlds = self.sample_worlds(view, self.samples);
        let scored = Self::score_worlds(view, &worlds, &candidates);
        Self::rank(&candidates, &scored, self.samples)
    }

    /// The ordered candidate actions for the current decision, the greedy
    /// incumbent first
    ///
    /// Mirrors the matching [`Strategy`] method's candidate set on this view,
    /// with one deliberate contraction: a knock's shed is not a real choice
    /// (dropping the largest deadwood is always the best knock), so the
    /// discard phase lists a single leading knock rather than one per shed.
    /// Empty when the seat has no real choice.
    fn hint_candidates(&self, view: &View<'_>) -> Vec<Candidate> {
        let shed = |label, action| Candidate {
            label,
            phase: SimPhase::Shed,
            action,
        };
        match view.phase() {
            Phase::Upcard => {
                let top = view.upcard().expect("the upcard offer has an upcard");
                let take = Candidate {
                    label: format!("take {top}"),
                    phase: SimPhase::Upcard,
                    action: RolloutAction::TakeDiscard,
                };
                let pass = Candidate {
                    label: "pass".to_string(),
                    phase: SimPhase::Upcard,
                    action: RolloutAction::Pass,
                };
                // Incumbent first, so the gate compares the challenger against
                // it exactly as `offer_upcard` does.
                if crate::heuristic::improves(view.hand(), top) {
                    vec![take, pass]
                } else {
                    vec![pass, take]
                }
            }
            Phase::Draw => {
                if !view.can_take_discard() {
                    // A forced stock draw is not a choice.
                    return Vec::new();
                }
                let top = view.upcard().expect("the pile is never empty on a draw");
                let stock = Candidate {
                    label: "draw stock".to_string(),
                    phase: SimPhase::Draw,
                    action: RolloutAction::DrawStock,
                };
                let pile = Candidate {
                    label: format!("take {top}"),
                    phase: SimPhase::Draw,
                    action: RolloutAction::TakeDiscard,
                };
                // Incumbent first, mirroring `choose_draw`.
                if crate::heuristic::improves(view.hand(), top) {
                    vec![pile, stock]
                } else {
                    vec![stock, pile]
                }
            }
            Phase::Discard => {
                let hand = view.hand();
                if deadwood(hand) == 0 && view.rules().big_gin_bonus.is_some() {
                    return vec![shed("big gin".to_string(), RolloutAction::BigGin)];
                }
                // The same greedy candidate ranking `play_turn` evaluates.
                let mut candidates: Vec<(Card, u8)> = hand
                    .iter()
                    .filter(|&card| Some(card) != view.taken_discard())
                    .map(|card| (card, deadwood(hand - card.into())))
                    .collect();
                candidates.sort_by_key(|&(card, rest)| (rest, u8::MAX - card.rank.deadwood()));
                candidates.truncate(MAX_CANDIDATES);

                let limit = view.knock_limit();
                let mut out = Vec::new();
                // The best knock leads, as the greedy incumbent; if even it
                // exceeds the limit, no shed can knock.
                if let Some(&(card, rest)) = candidates.first()
                    && rest <= limit
                {
                    let melds = best_melds(hand - card.into());
                    out.push(shed(
                        "knock".to_string(),
                        RolloutAction::Knock {
                            discard: card,
                            melds,
                        },
                    ));
                }
                for &(card, _) in &candidates {
                    out.push(shed(
                        format!("discard {card}"),
                        RolloutAction::Discard(card),
                    ));
                }
                out
            }
            _ => Vec::new(),
        }
    }

    /// Roll every candidate through the same `worlds` (common random numbers),
    /// returning per candidate its per-world equities and summed round points
    ///
    /// [`rank`](Self::rank) averages both the equities and the summed round
    /// points over the world count.
    fn score_worlds(
        view: &View<'_>,
        worlds: &[World],
        candidates: &[Candidate],
    ) -> Vec<(Vec<f64>, f64)> {
        let me = view.seat();
        let rules = view.rules();
        let standing = view.game_scores();
        candidates
            .iter()
            .map(|candidate| {
                let mut equities = Vec::with_capacity(worlds.len());
                let mut ev_sum = 0.0;
                for world in worlds {
                    let result = candidate
                        .action
                        .roll(Self::sim(view, world, candidate.phase));
                    equities.push(equity(result, me, standing, rules));
                    ev_sum += round_points(result, me, rules);
                }
                (equities, ev_sum)
            })
            .collect()
    }

    /// Reduce the scored candidates to assessments ranked by mean equity,
    /// flagging the bot's pick
    ///
    /// The pick is the greedy incumbent (`scored[0]`) unless a challenger's
    /// paired advantage clears the [`beats`] gate, in which case it is the
    /// largest such gain — exactly the deviation the [`Strategy`] methods make.
    fn rank(candidates: &[Candidate], scored: &[(Vec<f64>, f64)], samples: u32) -> Vec<Assessment> {
        let mean = |e: &[f64]| e.iter().sum::<f64>() / e.len() as f64;
        let defend = &scored[0].0;
        let recommended = (1..scored.len())
            .filter(|&i| beats(&scored[i].0, defend))
            .max_by(|&a, &b| mean(&scored[a].0).total_cmp(&mean(&scored[b].0)))
            .unwrap_or(0);
        let n = f64::from(samples);
        let mut out: Vec<Assessment> = candidates
            .iter()
            .zip(scored)
            .enumerate()
            .map(|(i, (candidate, (equities, ev_sum)))| Assessment {
                action: candidate.label.clone(),
                equity: equities.iter().sum::<f64>() / n,
                ev: ev_sum / n,
                recommended: i == recommended,
            })
            .collect();
        out.sort_by(|a, b| b.equity.total_cmp(&a.equity));
        out
    }
}

/// How many uniform hands [`MonteCarloBot::sample_worlds`] draws before
/// keeping the lowest-deadwood one, given the discard pile's current
/// length
///
/// Scales with the pile and never plateaus early — the 52-card deck
/// already bounds it below 16 by the last legal stock draw — so the
/// assumed opponent keeps improving for the whole round instead of
/// leveling off a third of the way through it.
const fn opponent_strength(pile_len: usize) -> usize {
    if pile_len < 2 { 1 } else { pile_len / 2 }
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

/// The value of `result` to `me` in the game standing at the `standing`
/// totals (`[mine, theirs]`): 1 for a result that wins the game, 0 for
/// one that loses it, otherwise affine in the signed round points
///
/// The result lands on the standing exactly as [`gin_rummy::Game::record`]
/// applies it: the winner banks [`RoundResult::points`] plus an immediate
/// box where [`Rules::immediate_boxes`] grants one.  Deferred boxes, the
/// game bonus, and shutout doubling only inflate the final tally — they
/// never decide who reaches [`Rules::game_target`] first — so they are
/// correctly absent.
///
/// Short of a clinch the value stays affine in round points, so `beats`
/// makes exactly the decisions the round-point objective made and the
/// bot deviates from its round game only when a rollout can actually end
/// the game: it takes the knock that clinches instead of milking a
/// bigger score, and it defends the round when losing it hands the
/// opponent the game.  Shaped utilities that also bend mid-game play — a
/// win-probability race over the points still needed — measured slightly
/// *weaker* over whole games (their distortion at level scores buys
/// nothing), and rolling whole games out instead would drown the
/// significance gate in cross-round variance.  A non-clinch gain is less
/// than the target by definition, so scaling by four targets pins every
/// mid-game value inside (¼, ¾), a guaranteed gap below a clinch and
/// above a loss.
fn equity(result: RoundResult, me: Player, standing: [u16; 2], rules: &Rules) -> f64 {
    let mut scores = standing;
    let mut points = 0.0;
    if let Some(winner) = result.winner() {
        let immediate = if rules.immediate_boxes {
            rules.box_bonus
        } else {
            0
        };
        let gain = result.points(rules).saturating_add(immediate);
        let side = usize::from(winner != me);
        scores[side] = scores[side].saturating_add(gain);
        points = if winner == me {
            f64::from(gain)
        } else {
            -f64::from(gain)
        };
    }
    // Mine first: both seats over the target is unreachable in a game,
    // where only one seat scores per round.
    if scores[0] >= rules.game_target {
        1.0
    } else if scores[1] >= rules.game_target {
        0.0
    } else {
        0.5 + points / (4.0 * f64::from(rules.game_target))
    }
}

/// The signed round points `result` wins `me`, the expected-value column of
/// [`MonteCarloBot::assess`]
///
/// Mirrors the `points` figure inside [`equity`] — the winner banks
/// [`RoundResult::points`] plus an immediate box where
/// [`Rules::immediate_boxes`] grants one — but returns the raw round points
/// rather than [`equity`]'s game-winning rescaling, so a solver can show
/// expected points beside the win-rate equity.
fn round_points(result: RoundResult, me: Player, rules: &Rules) -> f64 {
    let Some(winner) = result.winner() else {
        return 0.0;
    };
    let immediate = if rules.immediate_boxes {
        rules.box_bonus
    } else {
        0
    };
    let gain = result.points(rules).saturating_add(immediate);
    if winner == me {
        f64::from(gain)
    } else {
        -f64::from(gain)
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

        let worlds = self.sample_worlds(view, self.samples);
        let take = Self::equities(view, &worlds, SimPhase::Upcard, |mut sim| {
            sim.take_discard();
            sim.rollout()
        });
        let pass = Self::equities(view, &worlds, SimPhase::Upcard, |mut sim| {
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

        let worlds = self.sample_worlds(view, self.samples);
        let stock = Self::equities(view, &worlds, SimPhase::Draw, |mut sim| {
            sim.draw_stock();
            sim.rollout()
        });
        let pile = Self::equities(view, &worlds, SimPhase::Draw, |mut sim| {
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
        candidates.truncate(MAX_CANDIDATES);

        let worlds = self.sample_worlds(view, self.samples);
        let limit = view.knock_limit();
        let actions: Vec<(TurnAction, Vec<f64>)> = candidates
            .iter()
            .flat_map(|&(card, rest)| {
                let melds = best_melds(hand - card.into());
                let knock = (rest <= limit).then(|| {
                    let scores =
                        Self::equities(view, &worlds, SimPhase::Shed, |sim| sim.knock(card, melds));
                    (
                        TurnAction::Knock {
                            discard: card,
                            melds,
                        },
                        scores,
                    )
                });
                let discard = Self::equities(view, &worlds, SimPhase::Shed, |mut sim| {
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

        for world in bot.sample_worlds(&view, 32) {
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
    fn opponent_strength_keeps_growing_past_the_old_cap() {
        // The old formula flattened at 6 once the pile reached 12 cards;
        // a real opponent keeps improving long after that point, so the
        // replacement must keep climbing well past it.
        assert_eq!(opponent_strength(0), 1);
        assert_eq!(opponent_strength(12), 6);
        assert!(opponent_strength(24) > 6);
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
    fn equity_is_terminal_at_the_target() {
        let rules = Rules::default();
        let me = Player::One;
        let win = RoundResult::Knock {
            winner: me,
            margin: 15,
        };
        assert_eq!(equity(win, me, [90, 50], &rules), 1.0);

        let loss = RoundResult::Knock {
            winner: me.opponent(),
            margin: 15,
        };
        assert_eq!(equity(loss, me, [50, 90], &rules), 0.0);
    }

    #[test]
    fn equity_prices_immediate_boxes() {
        // 95 + 3 crosses 100 only with the palace box of 10.
        let me = Player::One;
        let result = RoundResult::Knock {
            winner: me,
            margin: 3,
        };
        assert_eq!(equity(result, me, [95, 95], &Rules::palace()), 1.0);

        let deferred = equity(result, me, [95, 95], &Rules::default());
        assert!(deferred > 0.5 && deferred < 1.0);
    }

    #[test]
    fn equity_orders_results_at_level_scores() {
        let rules = Rules::default();
        let me = Player::One;
        let gin = equity(
            RoundResult::Gin {
                winner: me,
                deadwood: 30,
            },
            me,
            [0, 0],
            &rules,
        );
        let knock = equity(
            RoundResult::Knock {
                winner: me,
                margin: 10,
            },
            me,
            [0, 0],
            &rules,
        );
        let dead = equity(RoundResult::Dead, me, [0, 0], &rules);
        let loss = equity(
            RoundResult::Knock {
                winner: me.opponent(),
                margin: 10,
            },
            me,
            [0, 0],
            &rules,
        );
        assert!(gin > knock && knock > dead && dead > loss);
        assert_eq!(dead, 0.5);
    }

    #[test]
    fn mid_game_equity_is_affine_in_round_points() {
        // Short of a clinch the standing shifts nothing: a dead round is
        // worth exactly 1/2, and a win is worth the same premium over it
        // from any standing — so mid-game decisions reduce to the
        // round-point objective.
        let rules = Rules::default();
        let me = Player::One;
        let win = RoundResult::Knock {
            winner: me,
            margin: 10,
        };
        assert_eq!(equity(RoundResult::Dead, me, [60, 20], &rules), 0.5);
        assert_eq!(
            equity(win, me, [60, 20], &rules),
            equity(win, me, [0, 0], &rules),
        );
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

    #[test]
    fn assess_ranks_candidates_and_flags_the_bots_pick() {
        let table = fixed_table();
        let seat = table.turn().expect("a fresh deal has a mover");
        let view = table.view(seat);

        // A solver and a chooser seeded alike sample identical worlds (the
        // rollout draws no randomness), so the flagged row must be the move
        // the bot actually plays.
        let mut solver = MonteCarloBot::new(StdRng::seed_from_u64(7)).samples(64);
        let mut chooser = MonteCarloBot::new(StdRng::seed_from_u64(7)).samples(64);

        let rows = solver.assess(&view);
        assert!(!rows.is_empty(), "the upcard offer is a real choice");

        // Equities are probabilities, and the table is ranked by them.
        for row in &rows {
            assert!((0.0..=1.0).contains(&row.equity));
        }
        assert!(rows.windows(2).all(|w| w[0].equity >= w[1].equity));

        // Exactly one recommendation, and it is the move the bot returns.
        assert_eq!(rows.iter().filter(|r| r.recommended).count(), 1);
        let picked = rows.iter().find(|r| r.recommended).expect("a flagged pick");
        let expected = match chooser.offer_upcard(&view) {
            UpcardAction::Take => format!("take {}", view.upcard().expect("an upcard offer")),
            UpcardAction::Pass => "pass".to_string(),
        };
        assert_eq!(picked.action, expected);
    }

    #[test]
    fn assess_reports_a_single_knock_at_a_discard() {
        // A knock's shed is forced — dropping the largest deadwood is always
        // the best knock — so the solver lists one knock row, not one per
        // shed, and it sheds that largest card.
        let two: Hand = "A23.456.789.2".parse().expect("a legal hand");
        let one: Hand = "TJ.TJ.TJ.3456".parse().expect("a legal hand");
        let upcard: Card = "QS".parse().expect("a card");
        // The non-dealer draws this off the stock (last card drawn first),
        // reaching an 11-card hand whose only loose cards are 2♠ and K♠.
        let king: Card = "KS".parse().expect("a card");
        let mut stock: Vec<Card> = (Hand::ALL - two - one - upcard.into() - king.into())
            .iter()
            .collect();
        stock.push(king);
        let round = Round::from_deal(Rules::default(), Player::One, [one, two], upcard, stock)
            .expect("a partitioned deck");
        let mut table = Table::new(round);

        // Both pass the upcard, forcing the non-dealer's stock draw and
        // landing them on the discard with nothing locked by a take.
        struct Passer;
        impl Strategy for Passer {
            fn offer_upcard(&mut self, _: &View<'_>) -> UpcardAction {
                UpcardAction::Pass
            }
            fn choose_draw(&mut self, _: &View<'_>) -> DrawAction {
                DrawAction::Stock
            }
            fn play_turn(&mut self, _: &View<'_>) -> TurnAction {
                unreachable!("the round stops at the discard")
            }
            fn choose_layoff(&mut self, _: &View<'_>) -> Option<Layoff> {
                None
            }
            fn name(&self) -> &str {
                "passer"
            }
        }
        while table.round().phase() != Phase::Discard {
            table
                .step(&mut Passer)
                .expect("a legal pass or forced draw");
        }

        let seat = table.turn().expect("the drawer is mid-turn");
        let mut solver = MonteCarloBot::new(StdRng::seed_from_u64(1)).samples(32);
        let rows = solver.assess(&table.view(seat));

        let knocks: Vec<_> = rows
            .iter()
            .filter(|r| r.action.starts_with("knock"))
            .collect();
        assert_eq!(knocks.len(), 1, "one knock row, not one per shed");
        assert_eq!(knocks[0].action, "knock");
    }
}
