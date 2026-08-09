//! [`MonteCarloBot`]: determinized Monte Carlo move selection

use crate::heuristic::greedy_layoff;
use crate::sim::{SeatPolicy, Sim, SimPhase};
use crate::value::WinTable;
use crate::{DealerRotation, DrawAction, Layoff, Strategy, TurnAction, UpcardAction, View};
use gin_rummy::{Card, Hand, Phase, Player, RoundResult, Rules, best_melds, deadwood};
use rand::{Rng, RngExt as _};

/// The world count of the first scoring batch; each later batch doubles the
/// evaluated total, so elimination checkpoints fall after 32, 64, 128, ...
/// worlds.  A decision of 32 samples or fewer is a single batch, identical
/// to an unbatched run.
const BATCH: usize = 32;

/// How the Monte Carlo rollouts model the opponent's draw decision
///
/// The model only shapes the forward simulation; nothing here reads any
/// information the [`View`] does not legally expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpponentModel {
    /// Take the pile card whenever it strictly lowers deadwood after the
    /// best shed — the greedy core's own rule, and the historical default.
    Eager,
    /// Take the pile card only when it lands in an immediate meld — the
    /// EAAI-2021 baseline's more conservative rule, for rollouts that
    /// model such an opponent faithfully.
    MeldOnly,
}

/// How the Monte Carlo equity values a mid-game round outcome
///
/// Short of a clinch a round result lands on the running game score; this
/// picks what that standing is worth to the bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GameValue {
    /// Value the standing affine in the round's signed points — the historical
    /// behavior, which prices the round correctly but treats a lead and a
    /// deficit as worth the same marginal point.
    Affine,
    /// Value the standing by the true probability of winning the game from
    /// it, a dynamic program over an empirical round-outcome model measured
    /// from greedy self-play.  This is the default, and gives the search
    /// score awareness — banking a lead, pressing a deficit, weighing the
    /// dealer rotation — where the affine value is flat.  Solved for the
    /// view's ruleset on first use and shared for the process; a ruleset
    /// with no measured model (anything but the built-in presets and the
    /// EAAI challenge variant) falls back to [`Affine`](Self::Affine).
    Table,
}

/// Tuning knobs for [`MonteCarloBot`]
///
/// Like [`HeuristicConfig`](crate::HeuristicConfig), the struct is
/// non-exhaustive: start from [`McConfig::default`] and adjust fields.
/// Every default is a *measured* setting rather than a taste: each one
/// won a whole-game sweep against fixed opponents, and changing one is a
/// strength change that owes the same measurement before it ships.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct McConfig {
    /// How many worlds each decision samples; more play stronger and
    /// slower.  See [`MonteCarloBot::samples`] for the measured
    /// strength/latency envelope of the default 128.
    pub samples: u32,
    /// The rollout knock threshold for the bot's own future self:
    /// continuations knock at residual deadwood ≤ `min(knock_limit,
    /// this)`.  The default 0 means a continuation banks only on gin, so
    /// the search compares *plans* — patient meld-building against an
    /// immediate knock — instead of pricing every line as if it ended at
    /// the first legal knock.  `u8::MAX`, the historical setting, knocks
    /// at the first legal chance and leaves multi-turn gin-hunting
    /// unrepresentable; on its own it cost 11.6 points of game win share
    /// against `marjj-v5-surrogate` (34.4% against 46.0% over 2000 games
    /// a side).  Raising it shortens rollouts, and is the lever to reach
    /// for when a decision budget is tight.
    pub rollout_knock_self: u8,
    /// The rollout knock threshold for the modeled opponent.  Holding
    /// *both* seats to the shipped heuristic's tuned threshold of 4
    /// measured clearly weaker (−6 and −8 points of decisive win rate on
    /// two 10 000-round seeds, −11 points over 300 games); the default
    /// `u8::MAX` keeps the urgent threat model that finding supports.
    /// Modeling a gin-camper (0) fares no better even against one: it
    /// cost about three points of game win share against
    /// `marjj-v5-surrogate`, which never knocks after turn three, because
    /// an opponent who never knocks makes every line look safe.
    pub rollout_knock_opponent: u8,
    /// How the modeled opponent decides to take the pile card.
    pub opponent_model: OpponentModel,
    /// The significance gate width in standard errors: the bot deviates
    /// from the greedy baseline action only when a challenger's paired
    /// advantage exceeds this many standard errors, and eliminates a
    /// challenger the baseline leads by the same bar.  Loosening it
    /// usually *weakens* the bot — deviating on noise plays worse than
    /// the baseline.
    pub gate_z: f64,
    /// How many lowest-deadwood sheds a discard decision weighs; the
    /// rest are never worth a rollout.
    pub max_candidates: usize,
    /// Scales the sampled opponent hands' plausibility bias, in percent
    /// of the base schedule (the best of `pile_len / 2` uniform draws
    /// keeps the lowest-deadwood hand).  The default 200 doubles the
    /// draws, worth about two points of game win share over 100 once the
    /// bot's own continuations are patient: a real opponent has been
    /// collecting melds all round, and pricing them as weaker than they
    /// are makes knocking look safe exactly where it is not.  0 samples
    /// uniformly random opponent hands.
    pub opponent_strength_percent: u32,
    /// How a mid-game round outcome is valued: [`GameValue::Table`], the
    /// default game-win value function that gives the search score
    /// awareness, or [`GameValue::Affine`], the historical round-point
    /// value.  See [`GameValue`].
    pub game_value: GameValue,
}

impl McConfig {
    /// The default configuration, every field a measured setting
    #[must_use]
    pub const fn new() -> Self {
        Self {
            samples: 128,
            rollout_knock_self: 0,
            rollout_knock_opponent: u8::MAX,
            opponent_model: OpponentModel::Eager,
            gate_z: 2.0,
            max_candidates: 4,
            opponent_strength_percent: 200,
            game_value: GameValue::Table,
        }
    }
}

impl Default for McConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// One determinized world: a concrete opponent hand and stock order
/// consistent with a [`View`]
struct World {
    opponent: Hand,
    /// Face-down draw order: the last element is drawn first
    stock: Vec<Card>,
}

/// One candidate action to score: the typed move a [`Strategy`] method would
/// return, paired with its rendered [`Assessment::action`] label
struct Candidate {
    /// The rendered [`Assessment::action`] label.
    label: String,
    /// The move itself — returned verbatim when this candidate is the pick,
    /// so the chooser and the solver read agree by construction.
    choice: Choice,
}

/// A typed candidate move, tagged by phase so the same value both drives a
/// rollout and is returned from the matching [`Strategy`] method
#[derive(Clone, Copy)]
enum Choice {
    /// Take or pass the initial upcard.
    Upcard(UpcardAction),
    /// Draw the stock or take the pile top.
    Draw(DrawAction),
    /// Discard, knock, or declare big gin.
    Turn(TurnAction),
}

impl Choice {
    /// The rollout phase this move acts at, passed to [`MonteCarloBot::sim`].
    fn phase(self) -> SimPhase {
        match self {
            Self::Upcard(_) => SimPhase::Upcard,
            Self::Draw(_) => SimPhase::Draw,
            Self::Turn(_) => SimPhase::Shed,
        }
    }

    /// Apply the move to a fresh rollout state and play it to a result.
    fn roll(self, mut sim: Sim) -> RoundResult {
        match self {
            Self::Upcard(UpcardAction::Take) | Self::Draw(DrawAction::TakeDiscard) => {
                sim.take_discard();
                sim.rollout()
            }
            Self::Upcard(UpcardAction::Pass) => {
                sim.pass();
                sim.rollout()
            }
            Self::Draw(DrawAction::Stock) => {
                sim.draw_stock();
                sim.rollout()
            }
            Self::Turn(TurnAction::BigGin(_)) => sim.big_gin(),
            Self::Turn(TurnAction::Knock { discard, melds }) => sim.knock(discard, melds),
            Self::Turn(TurnAction::Discard(card)) => {
                sim.discard(card).unwrap_or_else(|| sim.rollout())
            }
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
/// only when the paired samples show a statistically clear gain.  Worlds
/// are rolled in growing batches, and a challenger the incumbent already
/// statistically dominates is dropped at a batch boundary — once none
/// remain, the remaining worlds are never rolled at all — so an easy
/// decision costs a fraction of the full sample count.
///
/// The bot owns its random number generator, so a seeded generator makes
/// its play reproducible.
pub struct MonteCarloBot<R: Rng> {
    rng: R,
    config: McConfig,
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
    /// maximizes, so candidates rank by it.  A candidate the bot eliminated
    /// early averages over the worlds it saw before elimination rather than
    /// the full sample count.
    pub equity: f64,
    /// Mean signed round points the action wins the deciding seat: positive
    /// for a net gain, negative for a net loss.  Averaged over the same
    /// worlds as [`equity`](Self::equity).
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
        Self::with_config(rng, McConfig::new())
    }

    /// A bot with custom tuning
    ///
    /// [`McConfig::default`] reproduces [`MonteCarloBot::new`] exactly.
    #[must_use]
    pub const fn with_config(rng: R, config: McConfig) -> Self {
        Self { rng, config }
    }

    /// The bot's tuning knobs
    #[must_use]
    pub const fn config(&self) -> &McConfig {
        &self.config
    }

    /// Set how many worlds each decision samples
    ///
    /// More samples play stronger and slower.  At the default of 128 the
    /// bot wins about 64% of decisive rounds against the default
    /// [`HeuristicBot`] — which is tuned for whole-game play and so concedes
    /// single rounds — at roughly 10 ms per average turn in release builds
    /// (easy decisions stop at a fraction of the budget; a hard first
    /// discard, where every shed stays plausible, runs the full count for
    /// ~25 ms); 32 keeps a smaller edge at a quarter of the cost.  The
    /// `parallel` feature divides any of these by most of a machine's
    /// cores.
    ///
    /// [`HeuristicBot`]: crate::HeuristicBot
    #[must_use]
    pub const fn samples(mut self, samples: u32) -> Self {
        self.config.samples = samples;
        self
    }

    /// The per-seat rollout policies the configuration induces, mapped
    /// onto the viewing seat and its opponent.
    fn policies(&self, view: &View<'_>) -> [SeatPolicy; 2] {
        let mut policies = [SeatPolicy::default(); 2];
        let me = view.seat();
        policies[me as usize].knock_threshold = self.config.rollout_knock_self;
        let them = &mut policies[me.opponent() as usize];
        them.knock_threshold = self.config.rollout_knock_opponent;
        them.meld_only_draw = matches!(self.config.opponent_model, OpponentModel::MeldOnly);
        policies
    }

    /// The game-win value table for this ruleset, or `None` under
    /// [`GameValue::Affine`] and for a ruleset with no measured model
    ///
    /// The table is a `'static` reference into a process-wide cache, solved
    /// once per ruleset, so fetching it borrows nothing from the bot.
    fn value_table(
        &self,
        rules: &Rules,
        dealer_rotation: DealerRotation,
    ) -> Option<&'static WinTable> {
        matches!(self.config.game_value, GameValue::Table)
            .then(|| crate::value::table_for(rules, dealer_rotation))
            .flatten()
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
    /// partway through it.  Charging the pick a cold-card penalty as well —
    /// extra deadwood per hidden card adjoining one the opponent shed or
    /// passed, the heuristic's disinterest signal pointed the other way —
    /// measured flat (+0.2/−0.2 points on two 10 000-round seeds at mc:64),
    /// so the deadwood bias stands alone.
    fn sample_worlds(&mut self, view: &View<'_>, count: u32) -> Vec<World> {
        let unseen = view.unseen();
        let known = view.opponent_known();
        let missing = view.opponent_hand_len() - known.len();
        // At least one draw always happens, so 0 percent degrades to a
        // uniformly random hidden hand rather than an empty sample.
        let percent = self.config.opponent_strength_percent as usize;
        let strength = (opponent_strength(view.discard_pile().len()) * percent / 100).max(1);
        // One scratch pool for the whole decision.  Each hidden-hand draw is
        // a partial Fisher-Yates prefix, which is uniform from any starting
        // permutation, so the pool never needs rebuilding between draws.
        let mut pool: Vec<Card> = unseen.iter().collect();

        (0..count)
            .map(|_| {
                let hidden = (0..strength)
                    .map(|_| {
                        for i in 0..missing {
                            let j = self.rng.random_range(i..pool.len());
                            pool.swap(i, j);
                        }
                        pool[..missing].iter().copied().collect::<Hand>()
                    })
                    .min_by_key(|&hidden| deadwood(known | hidden))
                    .expect("at least one draw is always sampled");

                let mut stock: Vec<Card> = pool
                    .iter()
                    .copied()
                    .filter(|&card| !hidden.contains(card))
                    .collect();
                for i in (1..stock.len()).rev() {
                    let j = self.rng.random_range(0..=i);
                    stock.swap(i, j);
                }
                World {
                    opponent: known | hidden,
                    stock,
                }
            })
            .collect()
    }

    /// Instantiate one world as a rollout state, to act at `phase`
    fn sim(view: &View<'_>, world: &World, phase: SimPhase, policies: [SeatPolicy; 2]) -> Sim {
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
            policies,
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
        let policies = self.policies(view);
        let value = self.value_table(view.rules(), view.dealer_rotation());
        let worlds = self.sample_worlds(view, self.config.samples);
        let scored = Self::score_worlds(
            view,
            &worlds,
            &candidates,
            policies,
            self.config.gate_z,
            value,
        );
        Self::rank(&candidates, &scored, self.config.gate_z)
    }

    /// Score every candidate on freshly sampled worlds and return the move to
    /// play: the greedy incumbent (`candidates[0]`) unless a challenger clears
    /// the significance gate.  The shared core of the [`Strategy`] methods, so
    /// each is a thin wrapper over the same read [`assess`](Self::assess)
    /// surfaces; `candidates` must be non-empty.
    fn choose(&mut self, view: &View<'_>, candidates: &[Candidate]) -> Choice {
        let policies = self.policies(view);
        let value = self.value_table(view.rules(), view.dealer_rotation());
        let worlds = self.sample_worlds(view, self.config.samples);
        let scored = Self::score_worlds(
            view,
            &worlds,
            candidates,
            policies,
            self.config.gate_z,
            value,
        );
        candidates[recommended(&scored, self.config.gate_z)].choice
    }

    /// The ordered candidate moves for the current decision, the greedy
    /// incumbent first
    ///
    /// The single source of candidates for both the [`Strategy`] methods and
    /// the solver read, with one deliberate contraction: a knock's shed is not
    /// a real choice (dropping the largest deadwood is always the best knock),
    /// so the discard phase lists a single leading knock rather than one per
    /// shed.  Empty when the seat has no real choice.
    fn hint_candidates(&self, view: &View<'_>) -> Vec<Candidate> {
        let candidate = |label: String, choice: Choice| Candidate { label, choice };
        match view.phase() {
            Phase::Upcard => {
                let top = view.upcard().expect("the upcard offer has an upcard");
                let take = candidate(format!("take {top}"), Choice::Upcard(UpcardAction::Take));
                let pass = candidate("pass".to_string(), Choice::Upcard(UpcardAction::Pass));
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
                let stock = candidate("draw stock".to_string(), Choice::Draw(DrawAction::Stock));
                let pile = candidate(format!("take {top}"), Choice::Draw(DrawAction::TakeDiscard));
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
                    let choice = Choice::Turn(TurnAction::BigGin(best_melds(hand)));
                    return vec![candidate("big gin".to_string(), choice)];
                }
                // The same greedy shed ranking `play_turn` evaluates.
                let mut sheds: Vec<(Card, u8)> = hand
                    .iter()
                    .filter(|&card| Some(card) != view.taken_discard())
                    .map(|card| (card, deadwood(hand - card.into())))
                    .collect();
                sheds.sort_by_key(|&(card, rest)| (rest, u8::MAX - card.rank.deadwood()));
                sheds.truncate(self.config.max_candidates);

                let limit = view.knock_limit();
                let mut out = Vec::new();
                // The best knock leads, as the greedy incumbent; if even it
                // exceeds the limit, no shed can knock.
                if let Some(&(card, rest)) = sheds.first()
                    && rest <= limit
                {
                    let melds = best_melds(hand - card.into());
                    let knock = Choice::Turn(TurnAction::Knock {
                        discard: card,
                        melds,
                    });
                    out.push(candidate("knock".to_string(), knock));
                }
                for &(card, _) in &sheds {
                    let discard = Choice::Turn(TurnAction::Discard(card));
                    out.push(candidate(format!("discard {card}"), discard));
                }
                out
            }
            _ => Vec::new(),
        }
    }

    /// Roll candidates through the same `worlds` (common random numbers) in
    /// growing batches, eliminating challengers the incumbent already
    /// dominates, and return per candidate its per-world equities and summed
    /// round points
    ///
    /// A challenger is eliminated at a batch boundary when the incumbent's
    /// paired advantage over it clears the same [`beats`] gate a challenger
    /// must clear to be preferred; once every challenger is gone the
    /// incumbent wins by default and the remaining worlds are never rolled.
    /// Survivors always reach the full world count, so the final
    /// [`recommended`] read over them is exactly the unbatched one.  An
    /// eliminated candidate keeps the equities it accumulated: its paired
    /// mean against the incumbent is negative on that prefix, and [`beats`]
    /// zips to the shorter slice, so [`recommended`] rejects it with no
    /// special casing.
    fn score_worlds(
        view: &View<'_>,
        worlds: &[World],
        candidates: &[Candidate],
        policies: [SeatPolicy; 2],
        gate_z: f64,
        value: Option<&WinTable>,
    ) -> Vec<(Vec<f64>, f64)> {
        let me = view.seat();
        let rules = view.rules();
        let standing = view.game_scores();
        // Who deals next after a dead hand: the current round's dealer.
        let i_dealt = view.dealer() == me;
        let eval = |candidate: &Candidate, world: &World| {
            let sim = Self::sim(view, world, candidate.choice.phase(), policies);
            let result = candidate.choice.roll(sim);
            (
                equity(
                    result,
                    me,
                    standing,
                    rules,
                    value,
                    i_dealt,
                    view.dealer_rotation(),
                ),
                round_points(result, me, rules),
            )
        };

        let mut scored: Vec<(Vec<f64>, f64)> = vec![(Vec::new(), 0.0); candidates.len()];
        let mut alive: Vec<usize> = (1..candidates.len()).collect();
        let mut done = 0;
        while done < worlds.len() {
            let batch = &worlds[done..worlds.len().min(done + done.max(BATCH))];
            for &i in std::iter::once(&0).chain(&alive) {
                let candidate = &candidates[i];
                #[cfg(feature = "parallel")]
                let results: Vec<(f64, f64)> = {
                    use rayon::prelude::*;
                    batch
                        .par_iter()
                        .map(|world| eval(candidate, world))
                        .collect()
                };
                #[cfg(not(feature = "parallel"))]
                let results = batch.iter().map(|world| eval(candidate, world));

                // Reduced sequentially in world order in both builds, so a
                // parallel bot makes bit-identical decisions to a serial one.
                let (equities, ev_sum) = &mut scored[i];
                for (equity, points) in results {
                    equities.push(equity);
                    *ev_sum += points;
                }
            }
            done += batch.len();
            if done < worlds.len() {
                alive.retain(|&i| !beats(&scored[0].0, &scored[i].0, gate_z));
                if alive.is_empty() {
                    break;
                }
            }
        }
        scored
    }

    /// Reduce the scored candidates to assessments ranked by mean equity,
    /// flagging the bot's pick — the same index [`choose`](Self::choose)
    /// returns, so the solver read matches the move played
    ///
    /// Each candidate averages over the worlds it was actually rolled
    /// through, which is fewer than the sample count for a challenger
    /// [`score_worlds`](Self::score_worlds) eliminated early.
    fn rank(candidates: &[Candidate], scored: &[(Vec<f64>, f64)], gate_z: f64) -> Vec<Assessment> {
        let best = recommended(scored, gate_z);
        let mut out: Vec<Assessment> = candidates
            .iter()
            .zip(scored)
            .enumerate()
            .map(|(i, (candidate, (equities, ev_sum)))| {
                let n = equities.len() as f64;
                Assessment {
                    action: candidate.label.clone(),
                    equity: equities.iter().sum::<f64>() / n,
                    ev: ev_sum / n,
                    recommended: i == best,
                }
            })
            .collect();
        out.sort_by(|a, b| b.equity.total_cmp(&a.equity));
        out
    }
}

/// The index of the recommended candidate: the greedy incumbent (`scored[0]`)
/// unless a challenger's paired advantage clears the [`beats`] gate, in which
/// case the largest such gain
///
/// Shared by [`MonteCarloBot::choose`] and [`MonteCarloBot::rank`], so the
/// move the bot plays and the pick the solver flags never diverge.
fn recommended(scored: &[(Vec<f64>, f64)], gate_z: f64) -> usize {
    let mean = |e: &[f64]| e.iter().sum::<f64>() / e.len() as f64;
    let defend = &scored[0].0;
    (1..scored.len())
        .filter(|&i| beats(&scored[i].0, defend, gate_z))
        .max_by(|&a, &b| mean(&scored[a].0).total_cmp(&mean(&scored[b].0)))
        .unwrap_or(0)
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
/// the mean difference at least `gate_z` standard errors above zero
/// ([`McConfig::gate_z`], default 2 since several challengers get tested
/// per decision) — keeps only the deviations the samples actually support.
fn beats(challenger: &[f64], incumbent: &[f64], gate_z: f64) -> bool {
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
    mean > gate_z * (var / n).sqrt()
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
///
/// Under [`GameValue::Table`] a non-clinch standing is instead priced by
/// the game-win value function `value` — its true probability of winning
/// the game from the post-round scores under the table's dealer protocol.
/// A dead hand keeps its dealer under every supported protocol.  That value is
/// locally affine at level scores with the empirically correct slope, so it
/// agrees with the affine value there and diverges only where the game
/// recursion genuinely changes a point's worth.
fn equity(
    result: RoundResult,
    me: Player,
    standing: [u16; 2],
    rules: &Rules,
    value: Option<&WinTable>,
    i_dealt: bool,
    dealer_rotation: DealerRotation,
) -> f64 {
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
    } else if let Some(table) = value {
        let i_deal_next = result
            .winner()
            .map_or(i_dealt, |winner| match dealer_rotation {
                DealerRotation::WinnerDeals => winner == me,
                DealerRotation::AlternateAfterScoredRound => !i_dealt,
            });
        table.get(scores[0], scores[1], i_deal_next)
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
        let candidates = self.hint_candidates(view);
        match self.choose(view, &candidates) {
            Choice::Upcard(action) => action,
            _ => unreachable!("the upcard offer yields upcard choices"),
        }
    }

    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction {
        let candidates = self.hint_candidates(view);
        // The driver never consults a strategy on the forced stock draw, but
        // guard so a direct call cannot roll an empty candidate set.
        if candidates.is_empty() {
            return DrawAction::Stock;
        }
        match self.choose(view, &candidates) {
            Choice::Draw(action) => action,
            _ => unreachable!("the draw phase yields draw choices"),
        }
    }

    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        let hand = view.hand();
        if deadwood(hand) == 0 && view.rules().big_gin_bonus.is_some() {
            // Big gin scores at least as much as gin under every ruleset, and
            // is forced, so take it without a rollout (and without drawing
            // from the rng, keeping seeded play reproducible).
            return TurnAction::BigGin(best_melds(hand));
        }
        let candidates = self.hint_candidates(view);
        match self.choose(view, &candidates) {
            Choice::Turn(action) => action,
            _ => unreachable!("the discard phase yields turn choices"),
        }
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
        assert_eq!(
            equity(
                win,
                me,
                [90, 50],
                &rules,
                None,
                false,
                DealerRotation::WinnerDeals,
            ),
            1.0
        );

        let loss = RoundResult::Knock {
            winner: me.opponent(),
            margin: 15,
        };
        assert_eq!(
            equity(
                loss,
                me,
                [50, 90],
                &rules,
                None,
                false,
                DealerRotation::WinnerDeals,
            ),
            0.0
        );
    }

    #[test]
    fn equity_prices_immediate_boxes() {
        // 95 + 3 crosses 100 only with the palace box of 10.
        let me = Player::One;
        let result = RoundResult::Knock {
            winner: me,
            margin: 3,
        };
        assert_eq!(
            equity(
                result,
                me,
                [95, 95],
                &Rules::palace(),
                None,
                false,
                DealerRotation::WinnerDeals,
            ),
            1.0
        );

        let deferred = equity(
            result,
            me,
            [95, 95],
            &Rules::default(),
            None,
            false,
            DealerRotation::WinnerDeals,
        );
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
            None,
            false,
            DealerRotation::WinnerDeals,
        );
        let knock = equity(
            RoundResult::Knock {
                winner: me,
                margin: 10,
            },
            me,
            [0, 0],
            &rules,
            None,
            false,
            DealerRotation::WinnerDeals,
        );
        let dead = equity(
            RoundResult::Dead,
            me,
            [0, 0],
            &rules,
            None,
            false,
            DealerRotation::WinnerDeals,
        );
        let loss = equity(
            RoundResult::Knock {
                winner: me.opponent(),
                margin: 10,
            },
            me,
            [0, 0],
            &rules,
            None,
            false,
            DealerRotation::WinnerDeals,
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
        assert_eq!(
            equity(
                RoundResult::Dead,
                me,
                [60, 20],
                &rules,
                None,
                false,
                DealerRotation::WinnerDeals,
            ),
            0.5
        );
        assert_eq!(
            equity(
                win,
                me,
                [60, 20],
                &rules,
                None,
                false,
                DealerRotation::WinnerDeals,
            ),
            equity(
                win,
                me,
                [0, 0],
                &rules,
                None,
                false,
                DealerRotation::WinnerDeals,
            ),
        );
    }

    #[test]
    fn table_value_is_consulted_and_diverges_from_affine() {
        // `GameValue::Table` must actually reach the value table and price a
        // standing differently from the flat affine value — otherwise the
        // knob would be a silent no-op.
        let rules = crate::eaai_rules();
        let me = Player::One;
        let win = RoundResult::Knock {
            winner: me,
            margin: 10,
        };
        let table = crate::value::table_for(&rules, DealerRotation::WinnerDeals)
            .expect("the EAAI preset is baked");

        // From a commanding lead the affine value is flat while the table
        // knows the game is nearly clinched, so the two disagree.
        let affine = equity(
            win,
            me,
            [80, 10],
            &rules,
            None,
            false,
            DealerRotation::WinnerDeals,
        );
        let priced = equity(
            win,
            me,
            [80, 10],
            &rules,
            Some(table),
            false,
            DealerRotation::WinnerDeals,
        );
        assert_ne!(affine, priced);

        // And the table orders leads above deficits, as a probability must.
        let ahead = equity(
            win,
            me,
            [80, 10],
            &rules,
            Some(table),
            false,
            DealerRotation::WinnerDeals,
        );
        let behind = equity(
            win,
            me,
            [10, 80],
            &rules,
            Some(table),
            false,
            DealerRotation::WinnerDeals,
        );
        assert!(ahead > behind);
    }

    #[test]
    fn table_equity_uses_the_views_next_dealer_protocol() {
        let rules = crate::eaai_rules();
        let me = Player::One;
        let mine = RoundResult::Knock {
            winner: me,
            margin: 10,
        };
        let theirs = RoundResult::Knock {
            winner: me.opponent(),
            margin: 10,
        };
        let winner_table = crate::value::table_for(&rules, DealerRotation::WinnerDeals)
            .expect("the EAAI preset is baked");
        let alternate_table =
            crate::value::table_for(&rules, DealerRotation::AlternateAfterScoredRound)
                .expect("the EAAI preset is baked");

        assert_eq!(
            equity(
                mine,
                me,
                [0, 0],
                &rules,
                Some(winner_table),
                true,
                DealerRotation::WinnerDeals,
            ),
            winner_table.get(10, 0, true),
        );
        assert_eq!(
            equity(
                mine,
                me,
                [0, 0],
                &rules,
                Some(alternate_table),
                true,
                DealerRotation::AlternateAfterScoredRound,
            ),
            alternate_table.get(10, 0, false),
        );
        assert_eq!(
            equity(
                theirs,
                me,
                [0, 0],
                &rules,
                Some(alternate_table),
                false,
                DealerRotation::AlternateAfterScoredRound,
            ),
            alternate_table.get(0, 10, true),
        );
        assert_eq!(
            equity(
                RoundResult::Dead,
                me,
                [0, 0],
                &rules,
                Some(alternate_table),
                true,
                DealerRotation::AlternateAfterScoredRound,
            ),
            alternate_table.get(0, 0, true),
        );
    }

    #[test]
    fn table_equity_covers_every_winner_dealer_transition() {
        let rules = crate::eaai_rules();
        let me = Player::One;
        for rotation in [
            DealerRotation::WinnerDeals,
            DealerRotation::AlternateAfterScoredRound,
        ] {
            let table = crate::value::table_for(&rules, rotation).expect("the preset is baked");
            for i_dealt in [false, true] {
                for (result, scores, winner_is_me) in [
                    (
                        RoundResult::Knock {
                            winner: me,
                            margin: 7,
                        },
                        [7, 0],
                        Some(true),
                    ),
                    (
                        RoundResult::Knock {
                            winner: me.opponent(),
                            margin: 7,
                        },
                        [0, 7],
                        Some(false),
                    ),
                    (RoundResult::Dead, [0, 0], None),
                ] {
                    let next_dealer = winner_is_me.map_or(i_dealt, |winner_is_me| match rotation {
                        DealerRotation::WinnerDeals => winner_is_me,
                        DealerRotation::AlternateAfterScoredRound => !i_dealt,
                    });
                    assert_eq!(
                        equity(result, me, [0, 0], &rules, Some(table), i_dealt, rotation,),
                        table.get(scores[0], scores[1], next_dealer),
                    );
                }
            }
        }
    }

    #[test]
    fn table_bot_plays_and_repeats() {
        // A Table-configured bot exercises the value table end to end
        // (build, cache, lookup) without panicking, and stays deterministic.
        let table = knock_position();
        let seat = table.turn().expect("the drawer is mid-turn");
        let view = table.view(seat);
        let config = McConfig {
            samples: 32,
            game_value: GameValue::Table,
            ..McConfig::new()
        };
        let decide = |seed| {
            let mut bot = MonteCarloBot::with_config(StdRng::seed_from_u64(seed), config);
            let rows = bot.assess(&view);
            assert!(rows.iter().all(|r| (0.0..=1.0).contains(&r.equity)));
            rows.into_iter()
                .find(|r| r.recommended)
                .expect("a pick")
                .action
        };
        assert_eq!(decide(4), decide(4));
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
        assert!(!beats(&noisy, &base, 2.0));

        // A consistent advantage is.
        let better: Vec<f64> = base.iter().map(|x| x + 1.0).collect();
        assert!(beats(&better, &base, 2.0));
        assert!(!beats(&base, &better, 2.0));
        // Equality never beats.
        assert!(!beats(&base, &base, 2.0));
    }

    #[test]
    fn config_default_pins_the_measured_constants() {
        // Every default is a setting some sweep won; changing one is a
        // strength change and owes the measure-strength procedure, not
        // just an edit here.
        let config = McConfig::default();
        assert_eq!(config.samples, 128);
        assert_eq!(config.rollout_knock_self, 0);
        assert_eq!(config.rollout_knock_opponent, u8::MAX);
        assert_eq!(config.opponent_model, OpponentModel::Eager);
        assert!((config.gate_z - 2.0).abs() < f64::EPSILON);
        assert_eq!(config.max_candidates, 4);
        assert_eq!(config.opponent_strength_percent, 200);
        assert_eq!(config.game_value, GameValue::Table);
        // The two construction paths agree.
        assert_eq!(McConfig::new(), config);
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

    /// A table paused on a discard where the mover can knock: the non-dealer
    /// drew K♠ onto A♣2♣3♣ 4♦5♦6♦ 7♥8♥9♥ 2♠ after both seats passed the
    /// upcard, so shedding the king knocks at 2 deadwood with nothing locked
    /// by a take.
    fn knock_position() -> Table {
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
        table
    }

    #[test]
    fn assess_reports_a_single_knock_at_a_discard() {
        // A knock's shed is forced — dropping the largest deadwood is always
        // the best knock — so the solver lists one knock row, not one per
        // shed, and it sheds that largest card.
        let table = knock_position();
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

    #[test]
    fn seeded_pick_is_identical_across_serial_and_parallel_builds() {
        // The `parallel` feature must not change a single decision: batch
        // results are collected in world order and reduced sequentially in
        // both builds, so this exact pick is the answer in either one.  CI
        // runs the suite with and without the feature; a failure in only
        // one build means the parallel reduce stopped being order-exact.
        // Re-pin the expected action whenever sampling logic changes.
        let table = knock_position();
        let seat = table.turn().expect("the drawer is mid-turn");
        let mut bot = MonteCarloBot::new(StdRng::seed_from_u64(11)).samples(64);
        let rows = bot.assess(&table.view(seat));
        let pick = rows.iter().find(|r| r.recommended).expect("a flagged pick");
        // The patient default declines this legal knock; the pin is on the
        // pick being identical in both builds, not on which move it is.
        assert_eq!(pick.action, "discard K♠");
    }

    #[test]
    fn elimination_matches_the_full_read() {
        // Batched scoring must pick what an unbatched run over the same
        // worlds picks, spend strictly fewer rollouts doing it (the knock
        // dominates every plain shed here), and leave survivors' equities
        // bit-identical to the unbatched ones.
        let table = knock_position();
        let seat = table.turn().expect("the drawer is mid-turn");
        let view = table.view(seat);
        let mut bot = MonteCarloBot::new(StdRng::seed_from_u64(9)).samples(256);
        let candidates = bot.hint_candidates(&view);
        let worlds = bot.sample_worlds(&view, 256);
        let policies = bot.policies(&view);
        let batched =
            MonteCarloBot::<StdRng>::score_worlds(&view, &worlds, &candidates, policies, 2.0, None);

        let me = view.seat();
        let rules = view.rules();
        let standing = view.game_scores();
        let full: Vec<(Vec<f64>, f64)> = candidates
            .iter()
            .map(|candidate| {
                let mut equities = Vec::new();
                let mut ev_sum = 0.0;
                for world in &worlds {
                    let sim = MonteCarloBot::<StdRng>::sim(
                        &view,
                        world,
                        candidate.choice.phase(),
                        policies,
                    );
                    let result = candidate.choice.roll(sim);
                    equities.push(equity(
                        result,
                        me,
                        standing,
                        rules,
                        None,
                        false,
                        DealerRotation::WinnerDeals,
                    ));
                    ev_sum += round_points(result, me, rules);
                }
                (equities, ev_sum)
            })
            .collect();

        assert_eq!(recommended(&batched, 2.0), recommended(&full, 2.0));

        let rolled: usize = batched.iter().map(|(e, _)| e.len()).sum();
        let all: usize = full.iter().map(|(e, _)| e.len()).sum();
        assert!(
            rolled < all,
            "no challenger was eliminated: {rolled} of {all} rollouts"
        );

        for (b, f) in batched.iter().zip(&full) {
            if b.0.len() == worlds.len() {
                assert_eq!(
                    b.0, f.0,
                    "a survivor's equities must be unbatched-identical"
                );
            }
        }
    }
}
