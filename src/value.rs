//! A game-win value function for Monte Carlo equity (feature `rand`)
//!
//! [`MonteCarloBot`](crate::MonteCarloBot)'s older equity was affine in round
//! points short of a clinch, so it had no *score awareness*: a lead was worth
//! the same marginal point as a deficit, and the bot never banked a lead or
//! pressed a comeback the way [`HeuristicBot`](crate::HeuristicBot) does
//! through its tuned `score_awareness`.  This module supplies the
//! replacement — the true probability of winning the *game* from a pair of
//! level scores — which [`GameValue::Table`](crate::GameValue) uses to price
//! mid-game decisions by how they move the game rather than the round.
//!
//! The value is a dynamic program over an empirical round-outcome model.
//! Greedy self-play gives, per dealer role, how often the round is dead and
//! the distribution of points the winner banks; the DP then solves
//! `V(mine, theirs, i_deal_next)` — my probability of reaching
//! [`Rules::game_target`] first — by the game recursion the crate actually
//! plays: the round winner deals the next hand
//! ([`gin_rummy::Game::record`]), and a dead hand is redealt by the same
//! dealer, which closes into a self-loop solved in closed form.  Because
//! every decisive gain is at least one point under the presets this targets
//! (a tie is an undercut, never a zero-margin knock), scores strictly climb
//! toward the target and the DP fills bottom-up with no other cycle.
//!
//! The self-play half is *baked*, not sampled at run time.  Sampling the
//! model costs two seconds and solving the DP from it costs eight
//! milliseconds, and a two-second stall on a bot's first decision would be
//! felt by every interactive caller — worst of all in the browser front end,
//! which times its first hint to calibrate how many worlds the device can
//! afford.  The histograms below are therefore checked in, so construction
//! is only the DP.  They are a deterministic function of the rules (the
//! sampler is seeded by a constant), so the checked-in numbers are exactly
//! what sampling reproduces; `baked_matches_fresh_sampling` re-samples and
//! asserts it, which is also what catches an upstream mechanics change
//! silently invalidating them.  A ruleset we have no baked model for — the
//! browser's rules editor can build one — falls back to the affine value.
//!
//! Why this is not the shaped utility that measured *weaker* (recorded at
//! `equity` in `src/mc.rs`): that distortion bent mid-game play at level
//! scores where the round-point objective was already right.  `V` is
//! instead locally linear at level scores with the empirically correct
//! slope, so it reproduces round-point play there and deviates only where
//! the recursion says the marginal point value genuinely changes — near a
//! clinch, under a big lead or deficit, and across the dealer rotation.

use gin_rummy::Rules;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// A baked empirical round-outcome model for one built-in ruleset
///
/// `dealer_wins` and `nondealer_wins` are `(points, count)` histograms of
/// what the winner banks when the dealer, respectively the non-dealer, wins;
/// `dead` counts dead hands, and `total` is every sampled round.  A round
/// banking zero points is dropped when sampled, so `total` can exceed the
/// sum of the three.
struct Baked {
    dead: u32,
    total: u32,
    dealer_wins: &'static [(u16, u32)],
    nondealer_wins: &'static [(u16, u32)],
}

#[rustfmt::skip]
const MODERN: Baked = Baked {
    dead: 10,
    total: 20000,
    dealer_wins: &[
        (1,243), (2,299), (3,384), (4,424), (5,471), (6,491), (7,463), (8,448),
        (9,419), (10,419), (11,426), (12,420), (13,312), (14,325), (15,289), (16,286),
        (17,242), (18,256), (19,227), (20,214), (21,197), (22,149), (23,172), (24,140),
        (25,277), (26,247), (27,203), (28,135), (29,111), (30,96), (31,104), (32,80),
        (33,82), (34,87), (35,67), (36,80), (37,68), (38,79), (39,60), (40,47),
        (41,45), (42,50), (43,43), (44,44), (45,32), (46,46), (47,34), (48,37),
        (49,22), (50,22), (51,23), (52,17), (53,12), (54,18), (55,20), (56,13),
        (57,11), (58,14), (59,13), (60,9), (61,4), (62,5), (63,3), (64,1),
        (65,3), (66,3), (67,3), (69,3), (70,1), (71,3), (72,1), (75,1),
        (77,1), (79,1), (80,1), (82,1), (85,1), (88,1), (92,1), (94,1),
        (98,1),
    ],
    nondealer_wins: &[
        (1,258), (2,324), (3,427), (4,449), (5,462), (6,422), (7,454), (8,418),
        (9,425), (10,402), (11,412), (12,324), (13,356), (14,329), (15,313), (16,241),
        (17,265), (18,237), (19,220), (20,187), (21,175), (22,162), (23,158), (24,146),
        (25,249), (26,208), (27,172), (28,144), (29,126), (30,99), (31,85), (32,71),
        (33,76), (34,77), (35,57), (36,57), (37,73), (38,74), (39,59), (40,47),
        (41,54), (42,51), (43,54), (44,45), (45,51), (46,34), (47,39), (48,35),
        (49,23), (50,24), (51,28), (52,19), (53,17), (54,22), (55,17), (56,15),
        (57,17), (58,12), (59,8), (60,10), (61,8), (62,7), (63,5), (64,7),
        (65,6), (66,8), (67,5), (68,6), (69,1), (70,3), (72,2), (73,2),
        (74,1), (75,1), (76,1), (77,1), (83,1), (85,1), (86,1), (88,2),
        (91,1), (98,1),
    ],
};
#[rustfmt::skip]
const CLASSIC: Baked = Baked {
    dead: 10,
    total: 20000,
    dealer_wins: &[
        (1,243), (2,299), (3,384), (4,424), (5,471), (6,491), (7,463), (8,448),
        (9,419), (10,535), (11,533), (12,493), (13,348), (14,344), (15,298), (16,297),
        (17,249), (18,257), (19,227), (20,214), (21,197), (22,149), (23,172), (24,140),
        (25,161), (26,140), (27,130), (28,99), (29,92), (30,87), (31,116), (32,91),
        (33,99), (34,105), (35,78), (36,66), (37,56), (38,69), (39,51), (40,39),
        (41,43), (42,49), (43,44), (44,37), (45,32), (46,43), (47,31), (48,33),
        (49,25), (50,23), (51,21), (52,15), (53,9), (54,17), (55,17), (56,13),
        (57,12), (58,14), (59,9), (60,10), (61,4), (62,4), (63,2), (64,2),
        (66,4), (67,4), (69,2), (70,2), (72,1), (74,1), (75,1), (77,1),
        (80,1), (83,1), (87,1), (89,1), (93,1),
    ],
    nondealer_wins: &[
        (1,258), (2,324), (3,427), (4,449), (5,462), (6,422), (7,454), (8,418),
        (9,425), (10,520), (11,495), (12,398), (13,388), (14,350), (15,327), (16,246),
        (17,269), (18,238), (19,220), (20,187), (21,175), (22,162), (23,158), (24,146),
        (25,131), (26,125), (27,98), (28,112), (29,105), (30,85), (31,95), (32,81),
        (33,88), (34,90), (35,67), (36,55), (37,74), (38,67), (39,60), (40,49),
        (41,48), (42,41), (43,53), (44,34), (45,42), (46,31), (47,40), (48,38),
        (49,23), (50,25), (51,27), (52,19), (53,13), (54,18), (55,14), (56,13),
        (57,14), (58,9), (59,10), (60,11), (61,12), (62,7), (63,6), (64,5),
        (65,6), (66,3), (67,4), (68,6), (69,2), (70,2), (71,1), (72,1),
        (77,1), (78,1), (80,1), (81,1), (83,2), (86,1), (93,1),
    ],
};
#[rustfmt::skip]
const PALACE: Baked = Baked {
    dead: 10,
    total: 20000,
    dealer_wins: &[
        (11,243), (12,299), (13,384), (14,424), (15,471), (16,491), (17,463), (18,448),
        (19,419), (20,419), (21,426), (22,420), (23,312), (24,325), (25,289), (26,286),
        (27,242), (28,256), (29,227), (30,330), (31,304), (32,222), (33,208), (34,159),
        (35,170), (36,151), (37,137), (38,100), (39,92), (40,87), (41,93), (42,73),
        (43,81), (44,87), (45,67), (46,80), (47,68), (48,80), (49,60), (50,47),
        (51,45), (52,50), (53,43), (54,43), (55,32), (56,46), (57,34), (58,37),
        (59,22), (60,22), (61,23), (62,17), (63,12), (64,18), (65,20), (66,13),
        (67,11), (68,14), (69,14), (70,9), (71,4), (72,5), (73,3), (74,1),
        (75,2), (76,3), (77,3), (79,3), (80,1), (81,3), (82,1), (85,1),
        (87,1), (89,1), (90,1), (92,1), (95,1), (98,1), (102,1), (104,1),
        (108,1),
    ],
    nondealer_wins: &[
        (11,258), (12,324), (13,427), (14,449), (15,462), (16,422), (17,454), (18,418),
        (19,425), (20,402), (21,412), (22,324), (23,356), (24,329), (25,313), (26,241),
        (27,265), (28,237), (29,220), (30,305), (31,258), (32,236), (33,190), (34,167),
        (35,145), (36,130), (37,102), (38,113), (39,105), (40,85), (41,80), (42,67),
        (43,75), (44,77), (45,57), (46,57), (47,73), (48,74), (49,59), (50,47),
        (51,54), (52,51), (53,54), (54,45), (55,51), (56,34), (57,39), (58,36),
        (59,23), (60,24), (61,28), (62,20), (63,17), (64,21), (65,17), (66,15),
        (67,17), (68,11), (69,8), (70,10), (71,8), (72,7), (73,5), (74,7),
        (75,6), (76,8), (77,5), (78,6), (79,1), (80,3), (82,2), (83,2),
        (84,1), (85,1), (86,1), (87,1), (93,1), (95,1), (96,1), (98,2),
        (101,1), (108,1),
    ],
};
#[rustfmt::skip]
const EAAI: Baked = Baked {
    dead: 10,
    total: 20000,
    dealer_wins: &[
        (1,243), (2,299), (3,384), (4,424), (5,471), (6,491), (7,463), (8,448),
        (9,419), (10,419), (11,426), (12,420), (13,312), (14,325), (15,289), (16,286),
        (17,242), (18,256), (19,227), (20,214), (21,197), (22,149), (23,172), (24,140),
        (25,277), (26,247), (27,203), (28,135), (29,111), (30,96), (31,104), (32,80),
        (33,82), (34,87), (35,67), (36,80), (37,68), (38,80), (39,60), (40,47),
        (41,45), (42,50), (43,43), (44,43), (45,32), (46,46), (47,34), (48,37),
        (49,22), (50,22), (51,23), (52,17), (53,12), (54,18), (55,20), (56,13),
        (57,11), (58,14), (59,14), (60,9), (61,4), (62,5), (63,3), (64,1),
        (65,2), (66,3), (67,3), (69,3), (70,1), (71,3), (72,1), (75,1),
        (77,1), (79,1), (80,1), (82,1), (85,1), (88,1), (92,1), (94,1),
        (98,1),
    ],
    nondealer_wins: &[
        (1,258), (2,324), (3,427), (4,449), (5,462), (6,422), (7,454), (8,418),
        (9,425), (10,402), (11,412), (12,324), (13,356), (14,329), (15,313), (16,241),
        (17,265), (18,237), (19,220), (20,187), (21,175), (22,162), (23,158), (24,146),
        (25,249), (26,208), (27,172), (28,144), (29,126), (30,99), (31,85), (32,71),
        (33,76), (34,77), (35,57), (36,57), (37,73), (38,74), (39,59), (40,47),
        (41,54), (42,51), (43,54), (44,45), (45,51), (46,34), (47,39), (48,36),
        (49,23), (50,24), (51,28), (52,20), (53,17), (54,21), (55,17), (56,15),
        (57,17), (58,11), (59,8), (60,10), (61,8), (62,7), (63,5), (64,7),
        (65,6), (66,8), (67,5), (68,6), (69,1), (70,3), (72,2), (73,2),
        (74,1), (75,1), (76,1), (77,1), (83,1), (85,1), (86,1), (88,2),
        (91,1), (98,1),
    ],
};

/// The baked model for `rules`, or `None` for a ruleset we have not solved
///
/// Only the built-in presets are baked, plus the EAAI challenge variant the
/// benchmarks run under.  A caller handed anything else — the browser's
/// rules editor is the one that can produce it — gets `None` and keeps the
/// affine value rather than paying two seconds to sample a fresh model.
fn baked(rules: &Rules) -> Option<&'static Baked> {
    let eaai = || {
        let mut rules = Rules::new();
        rules.big_gin_bonus = None;
        rules
    };
    if *rules == Rules::new() {
        Some(&MODERN)
    } else if *rules == Rules::classic() {
        Some(&CLASSIC)
    } else if *rules == Rules::palace() {
        Some(&PALACE)
    } else if *rules == eaai() {
        Some(&EAAI)
    } else {
        None
    }
}

/// A solved game-win value function, indexed by both level scores and who
/// deals next
///
/// Built by [`table_for`] and shared for the process; queried through
/// [`WinTable::get`].
pub(crate) struct WinTable {
    target: usize,
    /// `probs[mine * target + theirs][i_deal]` = my probability of reaching
    /// the target first from level scores `(mine, theirs)`, both below the
    /// target, with `i_deal` = whether I deal the upcoming round.
    probs: Vec<[f64; 2]>,
}

impl WinTable {
    /// My game-win probability from level scores `(mine, theirs)`, with
    /// `i_deal` telling whether I deal the upcoming round
    ///
    /// Both scores must be below the target; a clinch is handled by the
    /// caller before it consults the table.
    pub(crate) fn get(&self, mine: u16, theirs: u16, i_deal: bool) -> f64 {
        let (mine, theirs) = (usize::from(mine), usize::from(theirs));
        debug_assert!(mine < self.target && theirs < self.target);
        self.probs[mine * self.target + theirs][usize::from(i_deal)]
    }
}

/// Solve `V(mine, theirs, i_deal_next)` by dynamic programming over a baked
/// outcome model
fn solve(target: u16, o: &Baked) -> WinTable {
    let n = usize::from(target);
    let total = f64::from(o.total);
    let p_dead = f64::from(o.dead) / total;
    debug_assert!(p_dead < 1.0, "some decisive rounds must be sampled");

    let mut probs = vec![[0.0f64; 2]; n * n];
    // Higher scores are closer to a clinch and depend on nothing lower, so
    // fill in decreasing score order.  A decisive round only ever moves the
    // winner strictly upward (every sampled gain is at least one point), so
    // each cell reads only already-filled higher cells; the dead-hand
    // self-loop is the sole cycle, and it is divided out in closed form.
    for m in (0..n).rev() {
        for t in (0..n).rev() {
            for d in 0..2 {
                let i_deal = d == 1;
                let mut acc = 0.0;
                // When the dealer wins: the dealer is me iff I deal.
                for &(g, c) in o.dealer_wins {
                    let (g, w) = (usize::from(g), f64::from(c) / total);
                    acc += w * if i_deal {
                        let m2 = m + g;
                        if m2 >= n { 1.0 } else { probs[m2 * n + t][1] }
                    } else {
                        let t2 = t + g;
                        if t2 >= n { 0.0 } else { probs[m * n + t2][0] }
                    };
                }
                // When the non-dealer wins: the non-dealer is me iff I do
                // not deal.
                for &(g, c) in o.nondealer_wins {
                    let (g, w) = (usize::from(g), f64::from(c) / total);
                    acc += w * if i_deal {
                        let t2 = t + g;
                        if t2 >= n { 0.0 } else { probs[m * n + t2][0] }
                    } else {
                        let m2 = m + g;
                        if m2 >= n { 1.0 } else { probs[m2 * n + t][1] }
                    };
                }
                // acc is the decisive-outcome mass; the dead self-loop keeps
                // the same dealer, so V = p_dead * V + acc.
                probs[m * n + t][d] = acc / (1.0 - p_dead);
            }
        }
    }

    WinTable { target: n, probs }
}

/// The process-wide table cache, keyed by rules
fn cache() -> &'static Mutex<HashMap<Rules, &'static WinTable>> {
    static CACHE: OnceLock<Mutex<HashMap<Rules, &'static WinTable>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The win-probability table for `rules`, solved once per ruleset per
/// process, or `None` where no baked model exists
///
/// The DP is a pure, deterministic function of the baked model, so the first
/// caller solves it and every later caller — across bots, seeds, and
/// threads — shares the result.  The lock is held only to solve or fetch,
/// never during a rollout, so the batched scorer sees no contention.
///
/// The solved table is intentionally leaked: there are four baked rulesets
/// and the cache lives for the whole process.
// ponytail: leak-per-ruleset; a bounded few tables, never freed by design.
pub(crate) fn table_for(rules: &Rules) -> Option<&'static WinTable> {
    let baked = baked(rules)?;
    let mut map = cache()
        .lock()
        .expect("the value-table cache is not poisoned");
    if let Some(&table) = map.get(rules) {
        return Some(table);
    }
    let table: &'static WinTable = Box::leak(Box::new(solve(rules.game_target, baked)));
    map.insert(*rules, table);
    Some(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Sim;
    use gin_rummy::{Card, Hand, Player, RoundResult};
    use rand::rngs::StdRng;
    use rand::{RngExt as _, SeedableRng as _};

    /// How many greedy self-play rounds seed the baked outcome model.
    const SAMPLES: usize = 20_000;

    /// A fixed seed for the outcome sampler, so a baked model is a
    /// deterministic function of the rules alone.
    const SEED: u64 = 0x_9E37_79B9_7F4A_7C15;

    /// Points above this fold into the top bucket; a decisive gain never
    /// reaches it (max is gin against a full high-card hand plus the bonus).
    const MAX_GAIN: usize = 256;

    /// The sampler that produced the baked histograms above
    ///
    /// Kept as test code because the baked data is what ships; this exists to
    /// regenerate it and to prove it still matches.
    /// One sampling run, in the shape the baked constants take
    struct Sampled {
        dead: u32,
        total: u32,
        dealer_wins: Vec<(u16, u32)>,
        nondealer_wins: Vec<(u16, u32)>,
    }

    fn sample_outcomes(rules: &Rules) -> Sampled {
        let mut rng = StdRng::seed_from_u64(SEED);
        let mut deck: Vec<Card> = Hand::ALL.iter().collect();
        let mut dealer_hist = [0u32; MAX_GAIN];
        let mut nondealer_hist = [0u32; MAX_GAIN];
        let mut dead = 0u32;

        for i in 0..SAMPLES {
            // A partial-to-full Fisher-Yates shuffle; the deck stays a
            // uniform permutation across iterations, so it is never rebuilt.
            for k in (1..deck.len()).rev() {
                let j = rng.random_range(0..=k);
                deck.swap(k, j);
            }
            let hands = [
                deck[..10].iter().copied().collect::<Hand>(),
                deck[10..20].iter().copied().collect::<Hand>(),
            ];
            let upcard = deck[20];
            let stock = deck[21..].to_vec();
            // Alternate the dealer so the two role histograms see equal deals.
            let dealer = Player::ALL[i & 1];
            let sim = Sim::from_deal(*rules, dealer, hands, upcard, stock);

            match sim.rollout() {
                RoundResult::Dead => dead += 1,
                result => {
                    let winner = result.winner().expect("a decisive result has a winner");
                    // The gain lands on the score exactly as `equity` and
                    // `gin_rummy::Game::record` apply it: round points plus
                    // an immediate box where the rules grant one.
                    let immediate = if rules.immediate_boxes {
                        rules.box_bonus
                    } else {
                        0
                    };
                    let gain = result.points(rules).saturating_add(immediate);
                    // A zero-point decisive round (a zero-margin knock, only
                    // possible without undercut-on-tie) is a self-loop the DP
                    // cannot represent; dropping the negligible fraction
                    // keeps the recursion well-founded on every ruleset.
                    // ponytail: drop-degenerate; revisit only for a preset
                    // that produces zero-margin knocks in bulk.
                    if gain == 0 {
                        continue;
                    }
                    let bucket = usize::from(gain).min(MAX_GAIN - 1);
                    if winner == dealer {
                        dealer_hist[bucket] += 1;
                    } else {
                        nondealer_hist[bucket] += 1;
                    }
                }
            }
        }

        let sparse = |hist: [u32; MAX_GAIN]| -> Vec<(u16, u32)> {
            hist.iter()
                .enumerate()
                .filter(|&(_, &c)| c > 0)
                .map(|(g, &c)| (g as u16, c))
                .collect()
        };
        let dealer_wins = sparse(dealer_hist);
        let nondealer_wins = sparse(nondealer_hist);
        let decisive: u32 = dealer_wins
            .iter()
            .chain(&nondealer_wins)
            .map(|&(_, c)| c)
            .sum();
        Sampled {
            dead,
            total: dead + decisive,
            dealer_wins,
            nondealer_wins,
        }
    }

    /// The EAAI challenge ruleset: modern bonuses, no big gin.
    fn eaai() -> Rules {
        let mut rules = Rules::new();
        rules.big_gin_bonus = None;
        rules
    }

    /// The checked-in histograms must be exactly what the sampler produces.
    /// This is the guard on an upstream mechanics change quietly making the
    /// baked model wrong: the rollout would drift, the sample would move,
    /// and the shipped table would keep pricing a game nobody plays.
    ///
    /// One preset is enough to catch drift — every preset shares the same
    /// rollout — and keeps the suite fast.  `cargo test regenerate_baked --
    /// --ignored --nocapture` reprints all four.
    #[test]
    fn baked_matches_fresh_sampling() {
        let rules = eaai();
        let fresh = sample_outcomes(&rules);
        assert_eq!(fresh.dead, EAAI.dead, "dead-hand count drifted");
        assert_eq!(fresh.total, EAAI.total, "sample total drifted");
        assert_eq!(
            fresh.dealer_wins, EAAI.dealer_wins,
            "dealer histogram drifted"
        );
        assert_eq!(
            fresh.nondealer_wins, EAAI.nondealer_wins,
            "non-dealer histogram drifted"
        );
    }

    /// Reprint every baked constant, for pasting back into this file after a
    /// deliberate mechanics change.  Ignored: it is a generator, not a check.
    #[test]
    #[ignore = "generator; run with --ignored --nocapture and paste the output"]
    fn regenerate_baked() {
        for (name, rules) in [
            ("MODERN", Rules::new()),
            ("CLASSIC", Rules::classic()),
            ("PALACE", Rules::palace()),
            ("EAAI", eaai()),
        ] {
            let s = sample_outcomes(&rules);
            // Eight pairs to a line, and `rustfmt::skip` so the formatter
            // does not explode them to one per line — a few hundred lines of
            // histogram would drown the module.
            let grid = |v: &Vec<(u16, u32)>| {
                v.chunks(8)
                    .map(|row| {
                        let cells: Vec<String> =
                            row.iter().map(|(g, c)| format!("({g},{c}),")).collect();
                        format!("        {}", cells.join(" "))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            println!("#[rustfmt::skip]");
            println!("const {name}: Baked = Baked {{");
            println!("    dead: {},", s.dead);
            println!("    total: {},", s.total);
            println!("    dealer_wins: &[");
            println!("{}", grid(&s.dealer_wins));
            println!("    ],");
            println!("    nondealer_wins: &[");
            println!("{}", grid(&s.nondealer_wins));
            println!("    ],");
            println!("}};");
        }
    }

    /// Every built-in ruleset resolves to a baked model, and anything else
    /// declines rather than stalling to sample one.
    #[test]
    fn only_built_in_rulesets_are_baked() {
        for rules in [Rules::new(), Rules::classic(), Rules::palace(), eaai()] {
            assert!(table_for(&rules).is_some(), "a built-in preset is baked");
        }
        let mut odd = Rules::new();
        odd.game_target = 137;
        assert!(table_for(&odd).is_none(), "an unknown ruleset declines");
    }

    /// A table is a genuine probability field: every entry lies in `[0, 1]`,
    /// a bigger lead is worth more, and a bigger deficit is worth less.
    #[test]
    fn table_is_a_monotone_probability_field() {
        let rules = eaai();
        let table = table_for(&rules).expect("the EAAI preset is baked");
        let n = rules.game_target;

        for mine in 0..n {
            for theirs in 0..n {
                for i_deal in [false, true] {
                    let v = table.get(mine, theirs, i_deal);
                    assert!((0.0..=1.0).contains(&v), "probability out of range: {v}");
                }
            }
        }

        // More of my own points never hurts; more of the opponent's never
        // helps.  Compared at a fixed dealer role.
        for theirs in 0..n - 1 {
            for i_deal in [false, true] {
                assert!(table.get(50, theirs, i_deal) >= table.get(50, theirs + 1, i_deal));
                assert!(table.get(theirs + 1, 50, i_deal) >= table.get(theirs, 50, i_deal));
            }
        }
    }

    /// The value function is score-aware where the affine equity is flat: a
    /// commanding lead is worth far more than an even game, and a deep
    /// deficit far less — the whole point of the table.
    #[test]
    fn table_prices_leads_and_deficits() {
        let table = table_for(&eaai()).expect("the EAAI preset is baked");

        let level = table.get(50, 50, true);
        let ahead = table.get(90, 10, true);
        let behind = table.get(10, 90, true);
        assert!(ahead > level && level > behind);
        // A near-even game is genuinely a toss-up.
        assert!(
            (level - 0.5).abs() < 0.1,
            "level score not near 0.5: {level}"
        );
    }
}
