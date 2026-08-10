//! Diagnose where `mc:128` loses points to the MARJJ v5 surrogate.
//!
//! This is a benchmark instrument, not another tournament surface.  It fixes
//! the opponent, rules, dealer protocol, and both candidate arms, then records
//! how the default and calibrated-hand searches turn rounds into points:
//!
//! ```console
//! cargo run --release --example marjj_diagnose -- \
//!   --game-pairs 2000 --seeds 7,8 \
//!   --json-out docs/marjj-m2.5-diagnostic.json
//! ```
//!
//! Trials are mirrored across seats and common-random-number paired across
//! arms.  The observer inspects the benchmark driver's full [`Round`] around
//! [`Table::step`], but strategies still receive only their legal [`View`].
//! Build without the `parallel` feature: trials already fan out across CPUs.
//!
//! [`Round`]: gin_rummy::Round
//! [`Table::step`]: gin_rummy_engine::Table::step
//! [`View`]: gin_rummy_engine::View

// Only the fixed MARJJ constructor is wanted here; the rest is benchmark
// support shared with the arena and conformance tests.
#[allow(dead_code)]
#[path = "support/arena_stats.rs"]
mod arena_stats;
#[allow(dead_code)]
#[path = "support/strong/mod.rs"]
mod strong;

use anyhow::{Context as _, Result, bail, ensure};
use arena_stats::{ExactPValue, RatioMoments, SignedRatioMoments};
use gin_rummy::{Game, Hand, Phase, Player, Round, RoundResult, Rules, deadwood};
use gin_rummy_engine::{DealerRotation, McConfig, MonteCarloBot, Strategy, Table, eaai_rules};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rayon::prelude::*;
use serde_json::{Map, Value, json};
use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const MIX: u64 = 0x9E37_79B9_7F4A_7C15;
const OPPONENT_STREAM: u64 = 0xD1B5_4A32_D192_ED03;
const CANDIDATE_STREAM: u64 = 0x94D0_49BB_1331_11EB;

const ARM_NAMES: [&str; 2] = ["default", "calibrated"];
const CHANNEL_NAMES: [&str; 7] = [
    "candidate_knock",
    "candidate_undercut",
    "candidate_gin",
    "marjj_knock",
    "marjj_undercut",
    "marjj_gin",
    "dead",
];
const TURN_BUCKETS: [&str; 4] = ["1-3", "4-6", "7-9", "10+"];
const SCORE_STATES: [&str; 3] = ["trailing", "tied", "leading"];
const ROUND_BUCKETS: [&str; 4] = ["1", "2", "3-4", "5+"];
const OPPONENT_DEADWOOD_BUCKETS: [&str; 6] = ["0", "1-3", "4-6", "7-10", "11-20", "21+"];
const DEADWOOD_DIFFERENCE_BUCKETS: [&str; 5] = ["<=-4", "-3..-1", "0", "1..3", ">=4"];
const KNOCK_OUTCOMES: [&str; 3] = ["knock_win", "undercut_loss", "gin_win"];

#[derive(Debug)]
struct Config {
    game_pairs: u32,
    seeds: Vec<u64>,
    json_out: PathBuf,
}

fn parse_args() -> Result<Config> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from(args: impl IntoIterator<Item = String>) -> Result<Config> {
    let mut game_pairs = 2_000;
    let mut seeds = vec![7, 8];
    let mut seeds_set = false;
    let mut json_out = None;
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = || args.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--game-pairs" => game_pairs = value()?.parse()?,
            "--seeds" => {
                let text = value()?;
                ensure!(!text.is_empty(), "--seeds needs a non-empty comma list");
                let mut parsed = Vec::new();
                for item in text.split(',') {
                    ensure!(!item.is_empty(), "--seeds contains an empty seed");
                    let seed = item.parse()?;
                    ensure!(!parsed.contains(&seed), "duplicate seed {seed}");
                    parsed.push(seed);
                }
                seeds = parsed;
                seeds_set = true;
            }
            "--json-out" => json_out = Some(PathBuf::from(value()?)),
            other => bail!("unknown flag {other:?} (--game-pairs/--seeds/--json-out)"),
        }
    }
    ensure!(game_pairs > 0, "--game-pairs must be greater than zero");
    ensure!(seeds_set || seeds == [7, 8], "the default seeds are fixed");
    Ok(Config {
        game_pairs,
        seeds,
        json_out: json_out.context("--json-out is required")?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arm {
    Default,
    Calibrated,
}

impl Arm {
    const ALL: [Self; 2] = [Self::Default, Self::Calibrated];

    const fn index(self) -> usize {
        self as usize
    }

    fn config(self) -> McConfig {
        let mut config = McConfig::default();
        config.hand_calibration = matches!(self, Self::Calibrated);
        config
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    CandidateKnock,
    CandidateUndercut,
    CandidateGin,
    MarjjKnock,
    MarjjUndercut,
    MarjjGin,
    Dead,
}

impl Channel {
    const ALL: [Self; 7] = [
        Self::CandidateKnock,
        Self::CandidateUndercut,
        Self::CandidateGin,
        Self::MarjjKnock,
        Self::MarjjUndercut,
        Self::MarjjGin,
        Self::Dead,
    ];

    const fn index(self) -> usize {
        self as usize
    }

    const fn name(self) -> &'static str {
        CHANNEL_NAMES[self.index()]
    }
}

fn channel(result: RoundResult, candidate: Player) -> Channel {
    match result {
        RoundResult::Dead => Channel::Dead,
        RoundResult::Knock { winner, .. } => {
            if winner == candidate {
                Channel::CandidateKnock
            } else {
                Channel::MarjjKnock
            }
        }
        RoundResult::Undercut { winner, .. } => {
            if winner == candidate {
                Channel::CandidateUndercut
            } else {
                Channel::MarjjUndercut
            }
        }
        RoundResult::Gin { winner, .. } | RoundResult::BigGin { winner, .. } => {
            if winner == candidate {
                Channel::CandidateGin
            } else {
                Channel::MarjjGin
            }
        }
        _ => unreachable!("the diagnostic supports every current round result"),
    }
}

const fn turn_bucket(turn: u32) -> usize {
    match turn {
        1..=3 => 0,
        4..=6 => 1,
        7..=9 => 2,
        _ => 3,
    }
}

const fn score_state(mine: u16, theirs: u16) -> usize {
    if mine < theirs {
        0
    } else if mine == theirs {
        1
    } else {
        2
    }
}

const fn round_bucket(round: u32) -> usize {
    match round {
        1 => 0,
        2 => 1,
        3..=4 => 2,
        _ => 3,
    }
}

const fn opponent_deadwood_bucket(value: u8) -> usize {
    match value {
        0 => 0,
        1..=3 => 1,
        4..=6 => 2,
        7..=10 => 3,
        11..=20 => 4,
        _ => 5,
    }
}

const fn deadwood_difference_bucket(value: i16) -> usize {
    match value {
        ..=-4 => 0,
        -3..=-1 => 1,
        0 => 2,
        1..=3 => 3,
        _ => 4,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CountPoints {
    count: u64,
    signed_points: i64,
}

impl CountPoints {
    fn record(&mut self, signed_points: i32) {
        self.count += 1;
        self.signed_points += i64::from(signed_points);
    }

    fn merge(mut self, other: Self) -> Self {
        self.count += other.count;
        self.signed_points += other.signed_points;
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ChannelStats {
    total: CountPoints,
    by_turn: [CountPoints; 4],
    by_score: [CountPoints; 3],
    by_round: [CountPoints; 4],
}

impl ChannelStats {
    fn record(&mut self, points: i32, turn: u32, score: usize, round: u32) {
        self.total.record(points);
        self.by_turn[turn_bucket(turn)].record(points);
        self.by_score[score].record(points);
        self.by_round[round_bucket(round)].record(points);
    }

    fn merge(mut self, other: Self) -> Self {
        self.total = self.total.merge(other.total);
        for index in 0..self.by_turn.len() {
            self.by_turn[index] = self.by_turn[index].merge(other.by_turn[index]);
        }
        for index in 0..self.by_score.len() {
            self.by_score[index] = self.by_score[index].merge(other.by_score[index]);
        }
        for index in 0..self.by_round.len() {
            self.by_round[index] = self.by_round[index].merge(other.by_round[index]);
        }
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KnockStats {
    attempts: u64,
    by_deadwood: [u64; 11],
    by_opponent_deadwood: [u64; 6],
    by_pre_layoff_difference: [u64; 5],
    outcomes: [u64; 3],
    layoff_cases: u64,
    layoff_reduction: u64,
}

impl KnockStats {
    fn merge(mut self, other: Self) -> Self {
        self.attempts += other.attempts;
        for index in 0..self.by_deadwood.len() {
            self.by_deadwood[index] += other.by_deadwood[index];
        }
        for index in 0..self.by_opponent_deadwood.len() {
            self.by_opponent_deadwood[index] += other.by_opponent_deadwood[index];
        }
        for index in 0..self.by_pre_layoff_difference.len() {
            self.by_pre_layoff_difference[index] += other.by_pre_layoff_difference[index];
        }
        for index in 0..self.outcomes.len() {
            self.outcomes[index] += other.outcomes[index];
        }
        self.layoff_cases += other.layoff_cases;
        self.layoff_reduction += other.layoff_reduction;
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DeclineStats {
    total_opportunities: u64,
    rounds_with_decline: u64,
    first_by_deadwood: [u64; 11],
    first_by_turn: [u64; 4],
    first_by_opponent_deadwood: [u64; 6],
    eventual_channel: [u64; 7],
}

impl DeclineStats {
    fn merge(mut self, other: Self) -> Self {
        self.total_opportunities += other.total_opportunities;
        self.rounds_with_decline += other.rounds_with_decline;
        for index in 0..self.first_by_deadwood.len() {
            self.first_by_deadwood[index] += other.first_by_deadwood[index];
        }
        for index in 0..self.first_by_turn.len() {
            self.first_by_turn[index] += other.first_by_turn[index];
        }
        for index in 0..self.first_by_opponent_deadwood.len() {
            self.first_by_opponent_deadwood[index] += other.first_by_opponent_deadwood[index];
        }
        for index in 0..self.eventual_channel.len() {
            self.eventual_channel[index] += other.eventual_channel[index];
        }
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Diagnostics {
    rounds: u64,
    channels: [ChannelStats; 7],
    knocks: KnockStats,
    declines: DeclineStats,
}

impl Diagnostics {
    fn merge(mut self, other: Self) -> Self {
        self.rounds += other.rounds;
        for index in 0..self.channels.len() {
            self.channels[index] = self.channels[index].merge(other.channels[index]);
        }
        self.knocks = self.knocks.merge(other.knocks);
        self.declines = self.declines.merge(other.declines);
        self
    }

    fn validate(&self) -> Result<()> {
        let finishes: u64 = self.channels.iter().map(|stats| stats.total.count).sum();
        ensure!(
            finishes == self.rounds,
            "finish channels account for every round"
        );

        for (name, stats) in CHANNEL_NAMES.iter().zip(self.channels) {
            ensure!(
                stats.by_turn.iter().map(|value| value.count).sum::<u64>() == stats.total.count,
                "{name} turn buckets balance"
            );
            ensure!(
                stats.by_score.iter().map(|value| value.count).sum::<u64>() == stats.total.count,
                "{name} score buckets balance"
            );
            ensure!(
                stats.by_round.iter().map(|value| value.count).sum::<u64>() == stats.total.count,
                "{name} round buckets balance"
            );
        }

        ensure!(
            self.knocks.by_deadwood.iter().sum::<u64>() == self.knocks.attempts,
            "candidate knock deadwood accounts for every attempt"
        );
        ensure!(
            self.knocks.by_opponent_deadwood.iter().sum::<u64>() == self.knocks.attempts,
            "opponent deadwood accounts for every candidate knock"
        );
        ensure!(
            self.knocks.by_pre_layoff_difference.iter().sum::<u64>() == self.knocks.attempts,
            "pre-layoff differences account for every candidate knock"
        );
        ensure!(
            self.knocks.outcomes.iter().sum::<u64>() == self.knocks.attempts,
            "candidate knock outcomes account for every attempt"
        );
        ensure!(
            self.knocks.outcomes[0] == self.channels[Channel::CandidateKnock.index()].total.count,
            "candidate knock wins agree with finish channels"
        );
        ensure!(
            self.knocks.outcomes[1] == self.channels[Channel::MarjjUndercut.index()].total.count,
            "failed candidate knocks agree with MARJJ undercuts"
        );
        ensure!(
            self.knocks.outcomes[2] == self.channels[Channel::CandidateGin.index()].total.count,
            "candidate gin wins agree with finish channels"
        );
        ensure!(
            self.declines.first_by_deadwood.iter().sum::<u64>()
                == self.declines.rounds_with_decline,
            "first declined deadwood accounts for every affected round"
        );
        ensure!(
            self.declines.first_by_turn.iter().sum::<u64>() == self.declines.rounds_with_decline,
            "first declined turn accounts for every affected round"
        );
        ensure!(
            self.declines.first_by_opponent_deadwood.iter().sum::<u64>()
                == self.declines.rounds_with_decline,
            "first declined opponent deadwood accounts for every affected round"
        );
        ensure!(
            self.declines.eventual_channel.iter().sum::<u64>() == self.declines.rounds_with_decline,
            "declined rounds have an eventual finish"
        );
        Ok(())
    }

    fn signed_points(&self) -> i64 {
        self.channels
            .iter()
            .map(|stats| stats.total.signed_points)
            .sum()
    }
}

#[derive(Debug, Clone, Copy)]
struct FirstDecline {
    deadwood: u8,
    turn: u32,
    opponent_deadwood: u8,
}

#[derive(Debug, Clone, Copy)]
struct PendingKnock {
    deadwood: u8,
    opponent_deadwood: u8,
}

fn minimum_legal_deadwood(hand: Hand, taken: Option<gin_rummy::Card>) -> u8 {
    hand.iter()
        .filter(|&card| Some(card) != taken)
        .map(|card| deadwood(hand - card.into()))
        .min()
        .expect("an eleven-card hand has a legal discard")
}

fn signed_points(result: RoundResult, candidate: Player, rules: &Rules) -> i32 {
    let Some(winner) = result.winner() else {
        return 0;
    };
    let points = i32::from(result.points(rules));
    if winner == candidate { points } else { -points }
}

fn play_traced_round(
    mut table: Table,
    strategies: [&mut dyn Strategy; 2],
    candidate: Player,
    game_round: u32,
    start_score_state: usize,
) -> Result<(Table, RoundResult, Diagnostics)> {
    let [one, two] = strategies;
    let mut turns = [0_u32; 2];
    let mut last_actor = None;
    let mut first_decline = None;
    let mut pending_knock = None;
    let mut diagnostics = Diagnostics::default();

    let result = loop {
        let seat = table.turn().context("a live round has a seat to act")?;
        let phase = table.round().phase();
        let candidate_decision = if phase == Phase::Discard && seat == candidate {
            let view = table.view(seat);
            Some((
                minimum_legal_deadwood(view.hand(), view.taken_discard()),
                deadwood(table.round().hand(candidate.opponent())),
                turns[seat as usize] + 1,
            ))
        } else {
            None
        };

        let finished = if seat == Player::One {
            table.step(&mut *one)?
        } else {
            table.step(&mut *two)?
        };

        if phase == Phase::Discard {
            turns[seat as usize] += 1;
            last_actor = Some(seat);
        }

        if let Some((available_deadwood, opponent_deadwood, turn)) = candidate_decision {
            if table.round().knocker() == Some(candidate) {
                let knock_deadwood = deadwood(table.round().hand(candidate));
                ensure!(
                    knock_deadwood <= 10,
                    "the fixed rules cap knock deadwood at ten"
                );
                pending_knock = Some(PendingKnock {
                    deadwood: knock_deadwood,
                    opponent_deadwood,
                });
            } else if available_deadwood <= table.round().knock_limit() {
                diagnostics.declines.total_opportunities += 1;
                first_decline.get_or_insert(FirstDecline {
                    deadwood: available_deadwood,
                    turn,
                    opponent_deadwood,
                });
            }
        }

        if let Some(result) = finished {
            break result;
        }
    };

    let finish_channel = channel(result, candidate);
    let ending_actor = table.round().knocker().or(last_actor).unwrap_or(candidate);
    let ending_turn = turns[ending_actor as usize].max(1);
    let points = signed_points(result, candidate, table.round().rules());
    diagnostics.rounds = 1;
    diagnostics.channels[finish_channel.index()].record(
        points,
        ending_turn,
        start_score_state,
        game_round,
    );

    if let Some(knock) = pending_knock {
        let stats = &mut diagnostics.knocks;
        stats.attempts += 1;
        stats.by_deadwood[usize::from(knock.deadwood)] += 1;
        stats.by_opponent_deadwood[opponent_deadwood_bucket(knock.opponent_deadwood)] += 1;
        let difference = i16::from(knock.opponent_deadwood) - i16::from(knock.deadwood);
        stats.by_pre_layoff_difference[deadwood_difference_bucket(difference)] += 1;
        match finish_channel {
            Channel::CandidateKnock => stats.outcomes[0] += 1,
            Channel::MarjjUndercut => stats.outcomes[1] += 1,
            Channel::CandidateGin => stats.outcomes[2] += 1,
            _ => bail!("a candidate knock ended in {}", finish_channel.name()),
        }
        if !matches!(finish_channel, Channel::CandidateGin) {
            let final_deadwood = deadwood(table.round().hand(candidate.opponent()));
            stats.layoff_cases += 1;
            stats.layoff_reduction +=
                u64::from(knock.opponent_deadwood.saturating_sub(final_deadwood));
        }
    }

    if let Some(decline) = first_decline {
        let stats = &mut diagnostics.declines;
        stats.rounds_with_decline += 1;
        stats.first_by_deadwood[usize::from(decline.deadwood)] += 1;
        stats.first_by_turn[turn_bucket(decline.turn)] += 1;
        stats.first_by_opponent_deadwood[opponent_deadwood_bucket(decline.opponent_deadwood)] += 1;
        stats.eventual_channel[finish_channel.index()] += 1;
    }

    diagnostics.validate()?;
    Ok((table, result, diagnostics))
}

#[derive(Debug, Clone, Copy, Default)]
struct GameStats {
    candidate_win: u32,
    candidate_points: u32,
    opponent_points: u32,
    diagnostics: Diagnostics,
}

fn play_game(
    arm: Arm,
    candidate: Player,
    first_dealer: Player,
    trial_seed: u64,
) -> Result<GameStats> {
    let rules = eaai_rules();
    let mut deal_rng = StdRng::seed_from_u64(trial_seed);
    let mut candidate_bot = MonteCarloBot::with_config(
        StdRng::seed_from_u64(trial_seed ^ CANDIDATE_STREAM),
        arm.config(),
    );
    let mut opponent = strong::make_bot("marjj-v5-surrogate", trial_seed ^ OPPONENT_STREAM)?
        .context("the fixed MARJJ spec exists")?;
    let mut game = Game::new(rules, first_dealer);
    let mut dealer = first_dealer;
    let mut diagnostics = Diagnostics::default();
    let mut game_round = 0;

    while !game.is_over() {
        game_round += 1;
        let scores = [game.score(Player::One), game.score(Player::Two)];
        let state = score_state(
            scores[candidate as usize],
            scores[candidate.opponent() as usize],
        );
        let table = Table::new(Round::deal(rules, dealer, &mut deal_rng))
            .scores(scores)
            .dealer_rotation(DealerRotation::AlternateAfterScoredRound);
        let seats: [&mut dyn Strategy; 2] = if candidate == Player::One {
            [&mut candidate_bot, &mut *opponent]
        } else {
            [&mut *opponent, &mut candidate_bot]
        };
        let (_, result, round_diagnostics) =
            play_traced_round(table, seats, candidate, game_round, state)?;
        diagnostics = diagnostics.merge(round_diagnostics);
        game.record(result)?;
        if result.winner().is_some() {
            dealer = dealer.opponent();
        }
    }

    let candidate_points = game.score(candidate);
    let opponent_points = game.score(candidate.opponent());
    ensure!(
        diagnostics.signed_points() == i64::from(candidate_points) - i64::from(opponent_points),
        "round channels equal the raw game-score margin"
    );
    Ok(GameStats {
        candidate_win: u32::from(game.winner() == Some(candidate)),
        candidate_points: u32::from(candidate_points),
        opponent_points: u32::from(opponent_points),
        diagnostics,
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct ArmStats {
    games: u64,
    wins: u64,
    candidate_points: u64,
    opponent_points: u64,
    diagnostics: Diagnostics,
    win_rate: RatioMoments,
    score_margin: SignedRatioMoments,
    sweeps: [u32; 2],
}

impl ArmStats {
    fn record_game(&mut self, game: GameStats) {
        self.games += 1;
        self.wins += u64::from(game.candidate_win);
        self.candidate_points += u64::from(game.candidate_points);
        self.opponent_points += u64::from(game.opponent_points);
        self.diagnostics = self.diagnostics.merge(game.diagnostics);
    }

    fn finish_trial(&mut self) {
        debug_assert_eq!(self.games, 2);
        self.win_rate.record(self.wins as u32, 2);
        let margin = self.candidate_points as i64 - self.opponent_points as i64;
        self.score_margin
            .record(i32::try_from(margin).expect("two games fit in i32"), 2);
        if self.wins == 2 {
            self.sweeps[0] = 1;
        } else if self.wins == 0 {
            self.sweeps[1] = 1;
        }
    }

    fn merge(mut self, other: Self) -> Self {
        self.games += other.games;
        self.wins += other.wins;
        self.candidate_points += other.candidate_points;
        self.opponent_points += other.opponent_points;
        self.diagnostics = self.diagnostics.merge(other.diagnostics);
        self.win_rate = self.win_rate.merge(other.win_rate);
        self.score_margin = self.score_margin.merge(other.score_margin);
        self.sweeps[0] += other.sweeps[0];
        self.sweeps[1] += other.sweeps[1];
        self
    }

    fn validate(&self, game_pairs: u32) -> Result<()> {
        ensure!(
            self.games == u64::from(game_pairs) * 2,
            "two games per pair"
        );
        ensure!(
            self.win_rate.clusters == game_pairs,
            "one win-rate cluster per pair"
        );
        ensure!(
            self.score_margin.clusters == game_pairs,
            "one margin cluster per pair"
        );
        ensure!(
            self.diagnostics.signed_points()
                == self.candidate_points as i64 - self.opponent_points as i64,
            "finish-channel points equal raw score margin"
        );
        self.diagnostics.validate()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ArmComparison {
    win_delta: SignedRatioMoments,
    score_margin_delta: SignedRatioMoments,
    win_signs: [u32; 2],
}

impl ArmComparison {
    fn record(default: ArmStats, calibrated: ArmStats) -> Self {
        let win_delta = calibrated.wins as i64 - default.wins as i64;
        let default_margin = default.candidate_points as i64 - default.opponent_points as i64;
        let calibrated_margin =
            calibrated.candidate_points as i64 - calibrated.opponent_points as i64;
        let mut comparison = Self::default();
        comparison
            .win_delta
            .record(i32::try_from(win_delta).expect("a pair has two games"), 2);
        comparison.score_margin_delta.record(
            i32::try_from(calibrated_margin - default_margin).expect("four game scores fit in i32"),
            2,
        );
        if win_delta > 0 {
            comparison.win_signs[0] = 1;
        } else if win_delta < 0 {
            comparison.win_signs[1] = 1;
        }
        comparison
    }

    fn merge(mut self, other: Self) -> Self {
        self.win_delta = self.win_delta.merge(other.win_delta);
        self.score_margin_delta = self.score_margin_delta.merge(other.score_margin_delta);
        self.win_signs[0] += other.win_signs[0];
        self.win_signs[1] += other.win_signs[1];
        self
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct SeedStats {
    arms: [ArmStats; 2],
    comparison: ArmComparison,
}

impl SeedStats {
    fn merge(mut self, other: Self) -> Self {
        for index in 0..self.arms.len() {
            self.arms[index] = self.arms[index].merge(other.arms[index]);
        }
        self.comparison = self.comparison.merge(other.comparison);
        self
    }

    fn validate(&self, game_pairs: u32) -> Result<()> {
        for arm in self.arms {
            arm.validate(game_pairs)?;
        }
        ensure!(
            self.comparison.win_delta.clusters == game_pairs,
            "one arm-comparison cluster per pair"
        );
        ensure!(
            self.comparison.score_margin_delta.clusters == game_pairs,
            "one arm-margin cluster per pair"
        );
        Ok(())
    }
}

fn trial(seed: u64, index: u32) -> Result<SeedStats> {
    let trial_seed = seed ^ u64::from(index).wrapping_mul(MIX);
    let first_dealer = if index % 4 < 2 {
        Player::One
    } else {
        Player::Two
    };
    let mut arms = [ArmStats::default(); 2];

    for arm in Arm::ALL {
        let stats = &mut arms[arm.index()];
        for candidate in Player::ALL {
            stats.record_game(play_game(arm, candidate, first_dealer, trial_seed)?);
        }
        stats.finish_trial();
    }

    Ok(SeedStats {
        arms,
        comparison: ArmComparison::record(arms[0], arms[1]),
    })
}

#[derive(Debug, Clone, Copy)]
struct SeedRun {
    seed: u64,
    stats: SeedStats,
    elapsed: Duration,
}

fn run_seed(seed: u64, game_pairs: u32) -> Result<SeedRun> {
    let start = Instant::now();
    let stats = (0..game_pairs)
        .into_par_iter()
        .map(|index| trial(seed, index))
        .try_reduce(SeedStats::default, |left, right| Ok(left.merge(right)))?;
    stats.validate(game_pairs)?;
    Ok(SeedRun {
        seed,
        stats,
        elapsed: start.elapsed(),
    })
}

fn labeled_counts(labels: &[&str], counts: &[u64]) -> Value {
    let values = labels
        .iter()
        .copied()
        .zip(counts.iter().copied())
        .map(|(label, count)| (label.to_owned(), json!(count)))
        .collect::<Map<_, _>>();
    Value::Object(values)
}

fn count_points_json(stats: CountPoints, games: u64) -> Value {
    json!({
        "count": stats.count,
        "signed_points": stats.signed_points,
        "signed_points_per_game": stats.signed_points as f64 / games.max(1) as f64,
    })
}

fn count_points_buckets(labels: &[&str], buckets: &[CountPoints], games: u64) -> Value {
    let values = labels
        .iter()
        .copied()
        .zip(buckets.iter().copied())
        .map(|(label, stats)| (label.to_owned(), count_points_json(stats, games)))
        .collect::<Map<_, _>>();
    Value::Object(values)
}

fn channel_stats_json(stats: ChannelStats, games: u64) -> Value {
    json!({
        "total": count_points_json(stats.total, games),
        "by_ending_actor_turn": count_points_buckets(&TURN_BUCKETS, &stats.by_turn, games),
        "by_starting_score": count_points_buckets(&SCORE_STATES, &stats.by_score, games),
        "by_game_round": count_points_buckets(&ROUND_BUCKETS, &stats.by_round, games),
    })
}

fn channels_json(diagnostics: Diagnostics, games: u64) -> Value {
    let values = Channel::ALL
        .into_iter()
        .map(|channel| {
            (
                channel.name().to_owned(),
                channel_stats_json(diagnostics.channels[channel.index()], games),
            )
        })
        .collect::<Map<_, _>>();
    Value::Object(values)
}

fn deadwood_counts(counts: &[u64; 11]) -> Value {
    Value::Object(
        counts
            .iter()
            .enumerate()
            .map(|(deadwood, &count)| (deadwood.to_string(), json!(count)))
            .collect(),
    )
}

fn diagnostics_json(diagnostics: Diagnostics, games: u64) -> Value {
    json!({
        "rounds": diagnostics.rounds,
        "finish_channels": channels_json(diagnostics, games),
        "candidate_knocks": {
            "attempts": diagnostics.knocks.attempts,
            "by_deadwood": deadwood_counts(&diagnostics.knocks.by_deadwood),
            "by_actual_opponent_deadwood": labeled_counts(
                &OPPONENT_DEADWOOD_BUCKETS,
                &diagnostics.knocks.by_opponent_deadwood,
            ),
            "by_pre_layoff_deadwood_difference": labeled_counts(
                &DEADWOOD_DIFFERENCE_BUCKETS,
                &diagnostics.knocks.by_pre_layoff_difference,
            ),
            "outcomes": labeled_counts(&KNOCK_OUTCOMES, &diagnostics.knocks.outcomes),
            "layoff_cases": diagnostics.knocks.layoff_cases,
            "layoff_deadwood_reduction": diagnostics.knocks.layoff_reduction,
        },
        "declined_legal_knocks": {
            "total_opportunities": diagnostics.declines.total_opportunities,
            "rounds_with_decline": diagnostics.declines.rounds_with_decline,
            "first_by_achievable_deadwood": deadwood_counts(
                &diagnostics.declines.first_by_deadwood,
            ),
            "first_by_turn": labeled_counts(
                &TURN_BUCKETS,
                &diagnostics.declines.first_by_turn,
            ),
            "first_by_actual_opponent_deadwood": labeled_counts(
                &OPPONENT_DEADWOOD_BUCKETS,
                &diagnostics.declines.first_by_opponent_deadwood,
            ),
            "eventual_finish": labeled_counts(
                &CHANNEL_NAMES,
                &diagnostics.declines.eventual_channel,
            ),
        },
    })
}

fn interval_json(interval: Option<arena_stats::Interval>) -> Value {
    interval.map_or(
        Value::Null,
        |interval| json!({"low": interval.low, "high": interval.high}),
    )
}

fn ratio_moments_json(moments: RatioMoments) -> Value {
    json!({
        "clusters": moments.clusters,
        "numerator": moments.numerator,
        "denominator": moments.denominator,
        "numerator_sq": moments.numerator_sq,
        "numerator_denominator": moments.numerator_denominator,
        "denominator_sq": moments.denominator_sq,
        "estimate": moments.estimate(),
        "cluster_ci95": interval_json(moments.cluster_interval()),
    })
}

fn signed_moments_json(moments: SignedRatioMoments) -> Value {
    json!({
        "clusters": moments.clusters,
        "numerator": moments.numerator,
        "denominator": moments.denominator,
        "numerator_sq": moments.numerator_sq,
        "numerator_denominator": moments.numerator_denominator,
        "denominator_sq": moments.denominator_sq,
        "estimate": moments.estimate(),
        "cluster_ci95": interval_json(moments.cluster_interval()),
    })
}

fn exact_p_json(value: ExactPValue) -> Value {
    json!({
        "numeric": value.as_f64(),
        "decimal": value.decimal(),
    })
}

fn arm_stats_json(stats: ArmStats) -> Value {
    let exact =
        ExactPValue::from_signs(stats.sweeps[0], stats.sweeps[1]).unwrap_or_else(ExactPValue::one);
    json!({
        "games": stats.games,
        "candidate_wins": stats.wins,
        "candidate_game_win_share": stats.wins as f64 / stats.games.max(1) as f64,
        "candidate_raw_points": stats.candidate_points,
        "marjj_raw_points": stats.opponent_points,
        "win_rate_moments": ratio_moments_json(stats.win_rate),
        "raw_score_margin_moments": signed_moments_json(stats.score_margin),
        "candidate_marjj_sweeps": stats.sweeps,
        "exact_sweep_sign_p_value": exact_p_json(exact),
        "diagnostics": diagnostics_json(stats.diagnostics, stats.games),
    })
}

fn channel_ranking(stats: SeedStats) -> Value {
    let games = stats.arms[0].games.max(1);
    let mut deltas = Channel::ALL
        .into_iter()
        .map(|channel| {
            let default = stats.arms[0].diagnostics.channels[channel.index()]
                .total
                .signed_points;
            let calibrated = stats.arms[1].diagnostics.channels[channel.index()]
                .total
                .signed_points;
            (channel, calibrated - default)
        })
        .collect::<Vec<_>>();
    deltas.sort_by_key(|&(_, delta)| Reverse(delta.unsigned_abs()));
    Value::Array(
        deltas
            .into_iter()
            .map(|(channel, delta)| {
                json!({
                    "channel": channel.name(),
                    "calibrated_minus_default_signed_points": delta,
                    "signed_points_per_game_delta": delta as f64 / games as f64,
                })
            })
            .collect(),
    )
}

fn comparison_json(stats: SeedStats) -> Value {
    let exact =
        ExactPValue::from_signs(stats.comparison.win_signs[0], stats.comparison.win_signs[1])
            .unwrap_or_else(ExactPValue::one);
    json!({
        "game_win_share_delta": signed_moments_json(stats.comparison.win_delta),
        "raw_score_margin_per_game_delta": signed_moments_json(
            stats.comparison.score_margin_delta,
        ),
        "trial_win_signs": {
            "calibrated": stats.comparison.win_signs[0],
            "default": stats.comparison.win_signs[1],
        },
        "exact_arm_sign_p_value": exact_p_json(exact),
        "finish_channel_ranking": channel_ranking(stats),
    })
}

fn arm_config_json(arm: Arm) -> Value {
    let config = arm.config();
    json!({
        "kind": "MonteCarloBot",
        "samples": config.samples,
        "rollout_knock_self": config.rollout_knock_self,
        "rollout_knock_opponent": config.rollout_knock_opponent,
        "opponent_model": format!("{:?}", config.opponent_model).to_lowercase(),
        "gate_z": config.gate_z,
        "max_candidates": config.max_candidates,
        "opponent_strength_percent": config.opponent_strength_percent,
        "hand_calibration": config.hand_calibration,
        "game_value": format!("{:?}", config.game_value).to_lowercase(),
    })
}

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn reproducibility_json() -> Value {
    let status = command_stdout("git", &["status", "--porcelain"]);
    json!({
        "git_head": command_stdout("git", &["rev-parse", "HEAD"]),
        "git_dirty": status.map(|status| !status.is_empty()),
        "rustc_vv": command_stdout("rustc", &["-Vv"]),
        "os": command_stdout("uname", &["-srvmo"]),
        "logical_threads": std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get),
    })
}

fn stats_json(stats: SeedStats) -> Value {
    json!({
        "arms": {
            ARM_NAMES[0]: arm_stats_json(stats.arms[0]),
            ARM_NAMES[1]: arm_stats_json(stats.arms[1]),
        },
        "calibrated_minus_default": comparison_json(stats),
    })
}

fn diagnosis_json(runs: &[SeedRun], pooled: SeedStats) -> Value {
    let ranking = channel_ranking(pooled);
    let leading = ranking
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("channel"))
        .and_then(Value::as_str)
        .expect("seven channels are ranked");
    let leading_index = CHANNEL_NAMES
        .iter()
        .position(|&channel| channel == leading)
        .expect("the leading channel is known");
    let seed_deltas = runs
        .iter()
        .map(|run| {
            run.stats.arms[1].diagnostics.channels[leading_index]
                .total
                .signed_points
                - run.stats.arms[0].diagnostics.channels[leading_index]
                    .total
                    .signed_points
        })
        .collect::<Vec<_>>();
    let agrees = seed_deltas.first().is_some_and(|first| {
        *first != 0
            && seed_deltas
                .iter()
                .all(|delta| delta.signum() == first.signum())
    });
    json!({
        "pooled_leading_channel": leading,
        "pooled_leading_channel_seed_deltas": seed_deltas,
        "leading_channel_agrees_across_seeds": agrees,
        "status": if agrees { "channel_stable" } else { "inconclusive" },
        "m3_status": if agrees { "blocked_pending_replan" } else { "blocked_pending_more_evidence" },
    })
}

fn report_json(runs: &[SeedRun], pooled: SeedStats, config: &Config, elapsed: Duration) -> Value {
    let rules = eaai_rules();
    json!({
        "schema": "gin-rummy-marjj-diagnostic/v1",
        "design": {
            "purpose": "M2.5 finish-channel diagnosis; not a publication strength panel",
            "game_pairs_per_arm_per_seed": config.game_pairs,
            "orientations_per_pair": 2,
            "games_per_trial": 4,
            "seeds": config.seeds,
            "common_random_numbers_across_arms": true,
            "optional_stopping": false,
            "rules": {
                "preset": "eaai",
                "knock_limit": rules.knock_limit,
                "gin_bonus": rules.gin_bonus,
                "undercut_bonus": rules.undercut_bonus,
                "undercut_on_tie": rules.undercut_on_tie,
                "big_gin_bonus": rules.big_gin_bonus,
                "box_bonus": rules.box_bonus,
                "immediate_boxes": rules.immediate_boxes,
                "game_bonus": rules.game_bonus,
                "game_target": rules.game_target,
                "oklahoma": null,
                "shutout": {"kind": "flat", "bonus": 0},
            },
            "dealer_rotation": "alternate_after_scored_round; dead hands retain dealer",
            "arms": {
                ARM_NAMES[0]: arm_config_json(Arm::Default),
                ARM_NAMES[1]: arm_config_json(Arm::Calibrated),
            },
            "opponent": {
                "spec": "marjj-v5-surrogate",
                "kind": "MarjjV5Surrogate",
                "initial_future_weight": 18,
                "discount": 0.9,
                "future_cards": 7,
                "tie_rng": "diagnostic_seeded_StdRng",
                "canonical_card_order": "C_H_S_D_then_rank",
                "canonical_meld_order": "meld_bitset",
            },
        },
        "reproducibility": reproducibility_json(),
        "elapsed_seconds": elapsed.as_secs_f64(),
        "runs": runs.iter().map(|run| json!({
            "seed": run.seed,
            "elapsed_seconds": run.elapsed.as_secs_f64(),
            "results": stats_json(run.stats),
        })).collect::<Vec<_>>(),
        "pooled": stats_json(pooled),
        "diagnosis": diagnosis_json(runs, pooled),
    })
}

fn write_json(path: &Path, report: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    let mut contents = serde_json::to_string_pretty(report)?;
    contents.push('\n');
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

fn main() -> Result<()> {
    let config = parse_args()?;
    let start = Instant::now();
    let mut runs = Vec::with_capacity(config.seeds.len());
    for &seed in &config.seeds {
        let run = run_seed(seed, config.game_pairs)?;
        eprintln!(
            "seed {seed}: {} pairs per arm in {:.1?}",
            config.game_pairs, run.elapsed,
        );
        runs.push(run);
    }
    let pooled = runs
        .iter()
        .fold(SeedStats::default(), |all, run| all.merge(run.stats));
    pooled.validate(config.game_pairs * config.seeds.len() as u32)?;
    let report = report_json(&runs, pooled, &config, start.elapsed());
    write_json(&config.json_out, &report)?;

    let default = pooled.arms[Arm::Default.index()];
    let calibrated = pooled.arms[Arm::Calibrated.index()];
    println!(
        "default {:.1}% vs calibrated {:.1}% over {} games per arm; wrote {}",
        100.0 * default.wins as f64 / default.games as f64,
        100.0 * calibrated.wins as f64 / calibrated.games as f64,
        default.games,
        config.json_out.display(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args<'a>(values: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
        values.iter().map(|value| (*value).to_owned())
    }

    #[test]
    fn parses_fixed_surface() {
        let config = parse_args_from(args(&[
            "--game-pairs",
            "12",
            "--seeds",
            "7,8",
            "--json-out",
            "out.json",
        ]))
        .expect("valid diagnostic arguments");
        assert_eq!(config.game_pairs, 12);
        assert_eq!(config.seeds, [7, 8]);
        assert_eq!(config.json_out, PathBuf::from("out.json"));
    }

    #[test]
    fn rejects_zero_duplicate_and_unknown_arguments() {
        for values in [
            &["--game-pairs", "0", "--json-out", "out.json"][..],
            &["--seeds", "7,7", "--json-out", "out.json"][..],
            &["--other", "1", "--json-out", "out.json"][..],
        ] {
            assert!(parse_args_from(args(values)).is_err());
        }
    }

    #[test]
    fn pins_both_arms_to_one_calibration_difference() {
        let default = Arm::Default.config();
        let calibrated = Arm::Calibrated.config();
        assert_eq!(default.samples, 128);
        assert_eq!(default.rollout_knock_self, 0);
        assert_eq!(default.opponent_strength_percent, 200);
        assert!(!default.hand_calibration);
        assert!(calibrated.hand_calibration);

        let mut restored = calibrated;
        restored.hand_calibration = false;
        assert_eq!(default, restored);
    }

    #[test]
    fn classifies_every_finish_from_the_candidate_view() {
        let candidate = Player::One;
        assert_eq!(channel(RoundResult::Dead, candidate), Channel::Dead);
        assert_eq!(
            channel(
                RoundResult::Knock {
                    winner: candidate,
                    margin: 1,
                },
                candidate,
            ),
            Channel::CandidateKnock,
        );
        assert_eq!(
            channel(
                RoundResult::Undercut {
                    winner: candidate,
                    margin: 1,
                },
                candidate,
            ),
            Channel::CandidateUndercut,
        );
        assert_eq!(
            channel(
                RoundResult::Gin {
                    winner: candidate.opponent(),
                    deadwood: 10,
                },
                candidate,
            ),
            Channel::MarjjGin,
        );
    }

    #[test]
    fn tracing_is_observational() {
        let rules = eaai_rules();
        let mut rng = StdRng::seed_from_u64(19);
        let round = Round::deal(rules, Player::One, &mut rng);
        let plain_round = round.clone();
        let traced_round = round;

        let mut plain_candidate = MonteCarloBot::new(StdRng::seed_from_u64(23)).samples(8);
        let mut plain_opponent = strong::make_bot("marjj-v5-surrogate", 29)
            .expect("construct the benchmark bot")
            .expect("the MARJJ spec exists");
        let mut plain =
            Table::new(plain_round).dealer_rotation(DealerRotation::AlternateAfterScoredRound);
        let plain_result = plain
            .play([&mut plain_candidate, &mut *plain_opponent])
            .expect("plain round finishes");

        let mut traced_candidate = MonteCarloBot::new(StdRng::seed_from_u64(23)).samples(8);
        let mut traced_opponent = strong::make_bot("marjj-v5-surrogate", 29)
            .expect("construct the benchmark bot")
            .expect("the MARJJ spec exists");
        let traced =
            Table::new(traced_round).dealer_rotation(DealerRotation::AlternateAfterScoredRound);
        let (traced, traced_result, diagnostics) = play_traced_round(
            traced,
            [&mut traced_candidate, &mut *traced_opponent],
            Player::One,
            1,
            score_state(0, 0),
        )
        .expect("traced round finishes");

        assert_eq!(plain_result, traced_result);
        assert_eq!(plain.round().phase(), traced.round().phase());
        assert_eq!(
            plain.round().hand(Player::One),
            traced.round().hand(Player::One),
        );
        assert_eq!(
            plain.round().hand(Player::Two),
            traced.round().hand(Player::Two),
        );
        assert_eq!(plain.round().discard_pile(), traced.round().discard_pile());
        assert_eq!(plain.round().stock(), traced.round().stock());
        diagnostics.validate().expect("trace accounting balances");
    }
}
