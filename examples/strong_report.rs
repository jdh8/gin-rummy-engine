//! Build the checked strong-opponent evidence bundle from arena JSON legs.
//!
//! The arena owns measurement and sufficient statistics; this helper owns
//! the fixed six-matchup panel, Holm correction, strength declarations, and
//! presentation.  It intentionally consumes JSON instead of scraping the
//! arena's human-readable output.

use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[allow(dead_code)]
#[path = "support/arena_stats.rs"]
mod arena_stats;

use arena_stats::{ExactPValue, RatioMoments, SignedRatioMoments, exact_sign_p_value};

const CANDIDATES: [&str; 3] = ["greedy", "mc:64", "mc:128"];
const OPPONENTS: [&str; 2] = ["gold-paper", "marjj-v5-surrogate"];
const SEEDS: [u64; 2] = [7, 8];
const PUBLICATION_ROUND_PAIRS: u64 = 4_000;
const PUBLICATION_GAME_PAIRS: u64 = 3_000;
const SMOKE_PAIRS: u64 = 20;
const FLOAT_TOLERANCE: f64 = 1e-12;
const CONFORMANCE_SCHEMA: &str = "gin-rummy-strong-conformance/v1";
const GOLD_COMMIT: &str = "3b2f5b7866d27234647c5833497c12ca1a2afde9";
const GOLD_SHA256: &str = "88a5ed62638de8c45c0a679c42cd2b05656b93336af9760905d77af04d1e7bca";
const MARJJ_COMMIT: &str = "5d1f00c1dff5380021785c8146d039a11efcabc3";
const MARJJ_SHA256: &str = "df6d4db2476ea35ee193258eec12f4925e1ea4d0fb703283fea3b1d4f82b9a4f";
const EAAI_COMMIT: &str = "559c712516e3b0fd6b908864acd141e254d94f39";
// Update only after the pinned upstream conformance workflow passes against
// this exact native adapter and probe corpus.
const CONFORMED_NATIVE_SOURCE_SHA256: &str =
    "cf0cef54c7d90643a826468b27794085656cfad706f29cda019f2a2f20afc88a";
const CONFORMANCE_SOURCE_FILES: [&str; 8] = [
    "contrib/strong-conformance/MarjjTrace.java",
    "contrib/strong-conformance/gold_probe.py",
    "examples/support/strong/gold.rs",
    "examples/support/strong/marjj.rs",
    "examples/support/strong/melds.rs",
    "examples/support/strong/mod.rs",
    "scripts/check-strong-conformance.sh",
    "tests/strong_conformance.rs",
];

struct Config {
    inputs: Vec<PathBuf>,
    json_out: PathBuf,
    markdown_out: PathBuf,
    round_pairs: u64,
    game_pairs: u64,
    smoke: bool,
    conformance_receipt: Option<PathBuf>,
}

#[derive(Default)]
struct Legs {
    rounds: Option<Value>,
    games: Option<Value>,
}

struct Matchup {
    candidate: String,
    opponent: String,
    rounds: Value,
    games: Value,
    raw_p: ExactPValue,
    holm_p: ExactPValue,
    seed_directions: Vec<i8>,
    verdict: &'static str,
}

fn parse_args() -> Result<Config> {
    let mut inputs = Vec::new();
    let mut json_out = None;
    let mut markdown_out = None;
    let mut round_pairs = None;
    let mut game_pairs = None;
    let mut smoke = false;
    let mut conformance_receipt = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        let mut value = || {
            args.next()
                .with_context(|| format!("{argument} needs a value"))
        };
        match argument.as_str() {
            "--json-out" => json_out = Some(PathBuf::from(value()?)),
            "--markdown-out" => markdown_out = Some(PathBuf::from(value()?)),
            "--round-pairs" => round_pairs = Some(value()?.parse()?),
            "--game-pairs" => game_pairs = Some(value()?.parse()?),
            "--smoke" => smoke = true,
            "--conformance-receipt" => conformance_receipt = Some(PathBuf::from(value()?)),
            flag if flag.starts_with('-') => bail!("unknown flag {flag:?}"),
            path => inputs.push(PathBuf::from(path)),
        }
    }
    ensure!(
        inputs.len() == 12,
        "expected 12 arena JSON legs, got {}",
        inputs.len()
    );
    let config = Config {
        inputs,
        json_out: json_out.context("--json-out is required")?,
        markdown_out: markdown_out.context("--markdown-out is required")?,
        round_pairs: round_pairs.context("--round-pairs is required")?,
        game_pairs: game_pairs.context("--game-pairs is required")?,
        smoke,
        conformance_receipt,
    };
    let expected = if config.smoke {
        (SMOKE_PAIRS, SMOKE_PAIRS)
    } else {
        (PUBLICATION_ROUND_PAIRS, PUBLICATION_GAME_PAIRS)
    };
    ensure!(
        (config.round_pairs, config.game_pairs) == expected,
        "{} report requires {}/{} round/game pairs per seed, got {}/{}",
        if config.smoke { "smoke" } else { "publication" },
        expected.0,
        expected.1,
        config.round_pairs,
        config.game_pairs
    );
    Ok(config)
}

fn pointer<'a>(document: &'a Value, path: &str) -> Result<&'a Value> {
    document
        .pointer(path)
        .with_context(|| format!("JSON has no {path}"))
}

fn string_at<'a>(document: &'a Value, path: &str) -> Result<&'a str> {
    pointer(document, path)?
        .as_str()
        .with_context(|| format!("{path} is not a string"))
}

fn u64_at(document: &Value, path: &str) -> Result<u64> {
    pointer(document, path)?
        .as_u64()
        .with_context(|| format!("{path} is not an unsigned integer"))
}

fn i64_at(document: &Value, path: &str) -> Result<i64> {
    pointer(document, path)?
        .as_i64()
        .with_context(|| format!("{path} is not an integer"))
}

fn f64_at(document: &Value, path: &str) -> Result<f64> {
    let value = pointer(document, path)?
        .as_f64()
        .with_context(|| format!("{path} is not numeric"))?;
    ensure!(value.is_finite(), "{path} is not finite");
    Ok(value)
}

fn exact_p_value_for(outcome: &Value) -> Result<ExactPValue> {
    let positive =
        u32::try_from(u64_at(outcome, "/comparison/sweeps/0")?).context("p1 sweeps exceed u32")?;
    let negative =
        u32::try_from(u64_at(outcome, "/comparison/sweeps/1")?).context("p2 sweeps exceed u32")?;
    Ok(exact_sign_p_value(positive, negative).unwrap_or_else(ExactPValue::one))
}

fn validate_p_value_field(
    outcome: &Value,
    numeric_name: &str,
    decimal_name: &str,
    expected: ExactPValue,
) -> Result<()> {
    let comparison = pointer(outcome, "/comparison")?;
    let numeric = comparison
        .get(numeric_name)
        .with_context(|| format!("comparison has no {numeric_name}"))?;
    match (numeric.as_f64(), expected.as_f64()) {
        (Some(actual), Some(expected)) => {
            ensure!(actual > 0.0, "{numeric_name} must be positive");
            ensure_close(actual, expected, numeric_name)?;
        }
        (None, None) if numeric.is_null() => {}
        // Version-1 arena files written before the decimal companion fields
        // used a false numeric zero when `f64` underflowed. Accept that only
        // as migration input; normalization below never writes it back out.
        (Some(0.0), None) if comparison.get(decimal_name).is_none() => {}
        _ => bail!("{numeric_name} does not match the recomputed exact p-value"),
    }
    if let Some(decimal) = comparison.get(decimal_name) {
        ensure!(
            decimal.as_str() == Some(expected.decimal().as_str()),
            "{decimal_name} does not match the recomputed exact p-value"
        );
    }
    Ok(())
}

fn normalize_outcome_p_values(outcome: &mut Value) -> Result<()> {
    let expected = exact_p_value_for(outcome)?;
    let comparison = outcome
        .get_mut("comparison")
        .and_then(Value::as_object_mut)
        .context("outcome comparison is not an object")?;
    let numeric = expected.as_f64().map_or(Value::Null, Value::from);
    let decimal = Value::String(expected.decimal());
    comparison.insert("exact_sign_p_value".to_owned(), numeric.clone());
    comparison.insert("exact_sign_p_value_decimal".to_owned(), decimal.clone());
    comparison.insert("primary_p_value".to_owned(), numeric);
    comparison.insert("primary_p_value_decimal".to_owned(), decimal);
    Ok(())
}

fn normalize_document_p_values(document: &mut Value) -> Result<()> {
    let runs = document
        .get_mut("runs")
        .and_then(Value::as_array_mut)
        .context("arena runs is not an array")?;
    for run in runs {
        normalize_outcome_p_values(run.get_mut("outcome").context("arena run has no outcome")?)?;
    }
    normalize_outcome_p_values(
        document
            .get_mut("pooled")
            .context("arena document has no pooled outcome")?,
    )
}

fn bool_at(document: &Value, path: &str) -> Result<bool> {
    pointer(document, path)?
        .as_bool()
        .with_context(|| format!("{path} is not Boolean"))
}

fn bot_spec<'a>(document: &'a Value, seat: &str) -> Result<&'a str> {
    let bot = pointer(document, &format!("/bots/{seat}"))?;
    bot.as_str()
        .or_else(|| bot.get("spec").and_then(Value::as_str))
        .with_context(|| format!("/bots/{seat} has no string spec"))
}

fn ensure_close(actual: f64, expected: f64, label: &str) -> Result<()> {
    let scale = actual.abs().max(expected.abs()).max(1.0);
    ensure!(
        (actual - expected).abs() <= FLOAT_TOLERANCE * scale,
        "{label} is {actual}, expected {expected}"
    );
    Ok(())
}

fn ensure_close_at(document: &Value, path: &str, expected: f64) -> Result<()> {
    ensure_close(f64_at(document, path)?, expected, path)
}

fn expected_bot_configuration(spec: &str) -> Result<Value> {
    let configuration = match spec {
        "greedy" => json!({
            "kind": "HeuristicBot",
            "knock_threshold": 4,
            "safety_weight": 1,
            "score_awareness": 40
        }),
        "mc:64" => monte_carlo_configuration(64),
        "mc:128" => monte_carlo_configuration(128),
        "gold-paper" => json!({
            "kind": "GoldPaperBot",
            "draw": "strict_minimum_deadwood_improvement",
            "ordinary_ties": "highest_pip_then_RLCard_S_H_D_C",
            "knock_ties": "RLCard_S_H_D_C",
            "knock_threshold": 10,
            "score_or_history_dependent": false
        }),
        "marjj-v5-surrogate" => json!({
            "kind": "MarjjV5Surrogate",
            "initial_future_weight": 18,
            "discount": 0.9,
            "future_cards": 7,
            "tie_rng": "arena_seeded_StdRng",
            "canonical_card_order": "C_H_S_D_then_rank",
            "canonical_meld_order": "meld_bitset"
        }),
        other => bail!("no predeclared configuration for bot {other:?}"),
    };
    Ok(configuration)
}

fn monte_carlo_configuration(samples: u64) -> Value {
    json!({
        "kind": "MonteCarloBot",
        "samples": samples,
        "rollout_knock_self": 255,
        "rollout_knock_opponent": 255,
        "opponent_model": "eager",
        "gate_z": 2.0,
        "max_candidates": 4,
        "opponent_strength_percent": 100,
        "game_value": "table"
    })
}

fn validate_bot(document: &Value, seat: &str, spec: &str) -> Result<()> {
    ensure!(bot_spec(document, seat)? == spec);
    ensure!(
        pointer(document, &format!("/bots/{seat}/configuration"))?
            == &expected_bot_configuration(spec)?,
        "raw {seat} configuration for {spec:?} differs from the predeclared configuration"
    );
    Ok(())
}

fn validate_rules(document: &Value) -> Result<()> {
    ensure!(string_at(document, "/rules/preset")? == "eaai");
    let rules = pointer(document, "/rules/values")?;
    let expected = json!({
        "knock_limit": 10,
        "oklahoma": null,
        "gin_bonus": 25,
        "big_gin_bonus": null,
        "undercut_bonus": 25,
        "undercut_on_tie": true,
        "box_bonus": 0,
        "immediate_boxes": false,
        "game_bonus": 0,
        "game_target": 100,
        "shutout": {"kind": "flat", "bonus": 0}
    });
    ensure!(
        rules == &expected,
        "arena leg does not use exact EAAI rules"
    );
    Ok(())
}

fn ratio_moments_at(document: &Value, path: &str) -> Result<RatioMoments> {
    let clusters = u32::try_from(u64_at(document, &format!("{path}/clusters"))?)
        .with_context(|| format!("{path}/clusters exceeds u32"))?;
    Ok(RatioMoments {
        clusters,
        numerator: u64_at(document, &format!("{path}/numerator"))?,
        denominator: u64_at(document, &format!("{path}/denominator"))?,
        numerator_sq: u64_at(document, &format!("{path}/numerator_sq"))?,
        numerator_denominator: u64_at(document, &format!("{path}/numerator_denominator"))?,
        denominator_sq: u64_at(document, &format!("{path}/denominator_sq"))?,
    })
}

fn signed_moments_at(document: &Value, path: &str) -> Result<SignedRatioMoments> {
    let clusters = u32::try_from(u64_at(document, &format!("{path}/clusters"))?)
        .with_context(|| format!("{path}/clusters exceeds u32"))?;
    Ok(SignedRatioMoments {
        clusters,
        numerator: i64_at(document, &format!("{path}/numerator"))?,
        denominator: u64_at(document, &format!("{path}/denominator"))?,
        numerator_sq: u64_at(document, &format!("{path}/numerator_sq"))?,
        numerator_denominator: i64_at(document, &format!("{path}/numerator_denominator"))?,
        denominator_sq: u64_at(document, &format!("{path}/denominator_sq"))?,
    })
}

fn validate_ratio_moments(
    document: &Value,
    path: &str,
    expected_clusters: u64,
    expected_numerator: u64,
    expected_denominator: u64,
    bounded: bool,
) -> Result<()> {
    let moments = ratio_moments_at(document, path)?;
    ensure!(
        u64::from(moments.clusters) == expected_clusters,
        "bad {path}/clusters"
    );
    ensure!(
        moments.numerator == expected_numerator,
        "bad {path}/numerator"
    );
    ensure!(
        moments.denominator == expected_denominator,
        "bad {path}/denominator"
    );
    let estimate = moments
        .estimate()
        .with_context(|| format!("{path} has a zero denominator"))?;
    ensure_close_at(document, &format!("{path}/estimate"), estimate)?;
    let interval = if bounded {
        moments.cluster_interval()
    } else {
        moments.cluster_interval_unbounded()
    }
    .with_context(|| format!("{path} cannot form a pair-cluster interval"))?;
    ensure_close_at(document, &format!("{path}/cluster_ci95/low"), interval.low)?;
    ensure_close_at(
        document,
        &format!("{path}/cluster_ci95/high"),
        interval.high,
    )?;
    Ok(())
}

fn validate_signed_moments(
    document: &Value,
    path: &str,
    expected_clusters: u64,
    expected_numerator: i64,
    expected_denominator: u64,
) -> Result<()> {
    let moments = signed_moments_at(document, path)?;
    ensure!(
        u64::from(moments.clusters) == expected_clusters,
        "bad {path}/clusters"
    );
    ensure!(
        moments.numerator == expected_numerator,
        "bad {path}/numerator"
    );
    ensure!(
        moments.denominator == expected_denominator,
        "bad {path}/denominator"
    );
    let estimate = moments
        .estimate()
        .with_context(|| format!("{path} has a zero denominator"))?;
    ensure_close_at(document, &format!("{path}/estimate"), estimate)?;
    let interval = moments
        .cluster_interval()
        .with_context(|| format!("{path} cannot form a pair-cluster interval"))?;
    ensure_close_at(document, &format!("{path}/cluster_ci95/low"), interval.low)?;
    ensure_close_at(
        document,
        &format!("{path}/cluster_ci95/high"),
        interval.high,
    )?;
    Ok(())
}

fn validate_win_interval(player: &Value, outcome: &Value, moments_path: &str) -> Result<()> {
    ensure!(
        string_at(player, "/win_ci95/method")? == "pair_cluster_normal",
        "win interval must use pair_cluster_normal"
    );
    ensure_close_at(
        player,
        "/win_ci95/low",
        f64_at(outcome, &format!("{moments_path}/cluster_ci95/low"))?,
    )?;
    ensure_close_at(
        player,
        "/win_ci95/high",
        f64_at(outcome, &format!("{moments_path}/cluster_ci95/high"))?,
    )?;
    Ok(())
}

fn validate_primary_fields(outcome: &Value) -> Result<()> {
    let expected = exact_p_value_for(outcome)?;
    validate_p_value_field(
        outcome,
        "exact_sign_p_value",
        "exact_sign_p_value_decimal",
        expected,
    )?;
    ensure!(
        string_at(outcome, "/comparison/primary_test")? == "exact_sweep_sign",
        "unexpected primary test"
    );
    validate_p_value_field(
        outcome,
        "primary_p_value",
        "primary_p_value_decimal",
        expected,
    )?;
    Ok(())
}

fn validate_sweep_consistency(
    outcome: &Value,
    mode: &str,
    trials: u64,
    wins: [u64; 2],
) -> Result<()> {
    let sweeps = [
        u64_at(outcome, "/comparison/sweeps/0")?,
        u64_at(outcome, "/comparison/sweeps/1")?,
    ];
    let sweep_total = sweeps[0]
        .checked_add(sweeps[1])
        .context("sweep count overflow")?;
    ensure!(sweep_total <= trials, "too many pair sweeps");
    let swept_wins = [
        sweeps[0].checked_mul(2).context("p1 sweep overflow")?,
        sweeps[1].checked_mul(2).context("p2 sweep overflow")?,
    ];
    ensure!(
        swept_wins[0] <= wins[0] && swept_wins[1] <= wins[1],
        "sweeps exceed wins"
    );

    if mode == "games" {
        let splits = [wins[0] - swept_wins[0], wins[1] - swept_wins[1]];
        ensure!(splits[0] == splits[1], "game split counts disagree by bot");
        ensure!(
            splits[0] == trials - sweep_total,
            "game sweeps and splits do not exhaust pair clusters"
        );
        ensure!(
            u64_at(outcome, "/comparison/paired_difference_sq_sum")?
                == sweep_total
                    .checked_mul(4)
                    .context("paired square-sum overflow")?,
            "game paired square-sum disagrees with sweeps"
        );
        let signed_sweeps = i64::try_from(sweeps[0]).context("p1 sweeps exceed i64")?
            - i64::try_from(sweeps[1]).context("p2 sweeps exceed i64")?;
        ensure!(
            i64_at(outcome, "/comparison/paired_difference_sum")?
                == signed_sweeps
                    .checked_mul(2)
                    .context("paired difference-sum overflow")?,
            "game paired difference-sum disagrees with sweeps"
        );
    }
    Ok(())
}

fn validate_outcome(
    outcome: &Value,
    mode: &str,
    trials: u64,
    candidate: &str,
    opponent: &str,
) -> Result<()> {
    let plays = trials.checked_mul(2).context("play count overflow")?;
    ensure!(u64_at(outcome, "/trials")? == trials);
    ensure!(u64_at(outcome, "/plays")? == plays);
    ensure!(u64_at(outcome, "/failures")? == 0);
    let decisive = u64_at(outcome, "/decisive")?;
    let players = pointer(outcome, "/players")?
        .as_array()
        .context("/players is not an array")?;
    ensure!(players.len() == 2, "outcome must have two players");
    let specs = [candidate, opponent];
    let mut wins = [0_u64; 2];
    let mut points = [0_u64; 2];
    for (index, spec) in specs.into_iter().enumerate() {
        let player = &players[index];
        ensure!(string_at(player, "/seat")? == format!("p{}", index + 1));
        ensure!(string_at(player, "/spec")? == spec);
        wins[index] = u64_at(player, "/wins")?;
        points[index] = u64_at(player, "/raw_points")?;
    }
    ensure!(
        wins[0] + wins[1] == decisive,
        "wins do not sum to decisive outcomes"
    );
    let denominator = if mode == "games" {
        ensure!(decisive == plays, "every completed game must be decisive");
        plays
    } else {
        let dead = u64_at(outcome, "/round_finishes/dead")?;
        ensure!(
            decisive + dead == plays,
            "round finishes do not sum to plays"
        );
        decisive
    };
    ensure!(
        denominator != 0,
        "cannot verify a zero-denominator win rate"
    );

    for index in 0..2 {
        let player = &players[index];
        ensure_close_at(player, "/win_rate", wins[index] as f64 / denominator as f64)?;
        let win_path = format!("/players/{index}/win_rate_moments");
        validate_ratio_moments(outcome, &win_path, trials, wins[index], denominator, true)?;
        validate_win_interval(player, outcome, &win_path)?;
        validate_ratio_moments(
            outcome,
            &format!("/players/{index}/point_rate_moments"),
            trials,
            points[index],
            plays,
            false,
        )?;
    }

    let finish_kinds = ["knock", "undercut", "gin", "big_gin"];
    let mut scored_rounds = 0_u64;
    for kind in finish_kinds {
        let player_total = u64_at(&players[0], &format!("/finishes/{kind}"))?
            + u64_at(&players[1], &format!("/finishes/{kind}"))?;
        ensure!(
            u64_at(outcome, &format!("/round_finishes/{kind}"))? == player_total,
            "round finish count for {kind} does not match player counts"
        );
        scored_rounds += player_total;
    }
    if mode == "rounds" {
        ensure!(
            scored_rounds == decisive,
            "scored round finishes do not match decisive count"
        );
    }

    let signed_win_margin = i64::try_from(wins[0]).context("p1 wins exceed i64")?
        - i64::try_from(wins[1]).context("p2 wins exceed i64")?;
    ensure!(
        i64_at(outcome, "/comparison/paired_difference_sum")? == signed_win_margin,
        "paired win-difference sum disagrees with wins"
    );
    ensure_close_at(
        outcome,
        "/comparison/rate_difference",
        signed_win_margin as f64 / denominator as f64,
    )?;
    validate_sweep_consistency(outcome, mode, trials, wins)?;
    validate_primary_fields(outcome)?;

    let point_margin = i64::try_from(points[0]).context("p1 points exceed i64")?
        - i64::try_from(points[1]).context("p2 points exceed i64")?;
    validate_signed_moments(
        outcome,
        "/comparison/point_margin_moments",
        trials,
        point_margin,
        plays,
    )?;
    Ok(())
}

fn ensure_u64_sum(pooled: &Value, runs: &[&Value], path: &str) -> Result<()> {
    let expected = runs.iter().try_fold(0_u64, |sum, run| {
        sum.checked_add(u64_at(run, path)?)
            .with_context(|| format!("overflow while summing {path}"))
    })?;
    ensure!(
        u64_at(pooled, path)? == expected,
        "pooled {path} is not additive"
    );
    Ok(())
}

fn ensure_i64_sum(pooled: &Value, runs: &[&Value], path: &str) -> Result<()> {
    let expected = runs.iter().try_fold(0_i64, |sum, run| {
        sum.checked_add(i64_at(run, path)?)
            .with_context(|| format!("overflow while summing {path}"))
    })?;
    ensure!(
        i64_at(pooled, path)? == expected,
        "pooled {path} is not additive"
    );
    Ok(())
}

fn validate_pooled_additivity(document: &Value) -> Result<()> {
    let runs = pointer(document, "/runs")?
        .as_array()
        .context("/runs is not an array")?;
    let outcomes = runs
        .iter()
        .map(|run| pointer(run, "/outcome"))
        .collect::<Result<Vec<_>>>()?;
    let pooled = pointer(document, "/pooled")?;
    let mut unsigned = [
        "/trials",
        "/plays",
        "/decisive",
        "/failures",
        "/round_finishes/knock",
        "/round_finishes/undercut",
        "/round_finishes/gin",
        "/round_finishes/big_gin",
        "/round_finishes/dead",
        "/comparison/paired_difference_sq_sum",
        "/comparison/sweeps/0",
        "/comparison/sweeps/1",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    for index in 0..2 {
        unsigned.push(format!("/players/{index}/wins"));
        unsigned.push(format!("/players/{index}/raw_points"));
        for kind in ["knock", "undercut", "gin", "big_gin"] {
            unsigned.push(format!("/players/{index}/finishes/{kind}"));
        }
        for family in ["win_rate_moments", "point_rate_moments"] {
            for field in [
                "clusters",
                "numerator",
                "denominator",
                "numerator_sq",
                "numerator_denominator",
                "denominator_sq",
            ] {
                unsigned.push(format!("/players/{index}/{family}/{field}"));
            }
        }
    }
    for field in ["clusters", "denominator", "numerator_sq", "denominator_sq"] {
        unsigned.push(format!("/comparison/point_margin_moments/{field}"));
    }
    for path in unsigned {
        ensure_u64_sum(pooled, &outcomes, &path)?;
    }
    for path in [
        "/comparison/paired_difference_sum",
        "/comparison/point_margin_moments/numerator",
        "/comparison/point_margin_moments/numerator_denominator",
    ] {
        ensure_i64_sum(pooled, &outcomes, path)?;
    }
    Ok(())
}

fn validate_reproducibility(document: &Value) -> Result<()> {
    let environment = pointer(document, "/reproducibility")?;
    ensure!(environment.is_object(), "/reproducibility is not an object");
    string_at(environment, "/source_sha256")?;
    u64_at(environment, "/source_file_count")?;
    string_at(environment, "/cargo_lock_sha256")?;
    string_at(environment, "/git_head")?;
    bool_at(environment, "/git_dirty")?;
    string_at(environment, "/rustc_vv")?;
    string_at(environment, "/os")?;
    string_at(environment, "/arch")?;
    u64_at(environment, "/logical_threads")?;
    Ok(())
}

fn validate_same_reproducibility(expected: &Value, actual: &Value) -> Result<()> {
    ensure!(
        actual == expected,
        "immutable reproducibility metadata differs across arena legs"
    );
    Ok(())
}

fn validate_document(
    document: &Value,
    mode: &str,
    pairs: u64,
    candidate: &str,
    opponent: &str,
) -> Result<()> {
    ensure!(string_at(document, "/schema")? == "gin-rummy-arena/v1");
    ensure!(string_at(document, "/mode")? == mode);
    ensure!(bool_at(document, "/paired")?);
    ensure!(u64_at(document, "/count_per_seed")? == pairs);
    ensure!(u64_at(document, "/failures")? == 0);
    ensure!(string_at(document, "/dealer_rotation")? == "alternate_after_scored_round");
    ensure!(
        string_at(document, "/points_metric")?
            == if mode == "games" {
                "raw_game_score"
            } else {
                "round_points"
            }
    );
    validate_rules(document)?;
    validate_bot(document, "p1", candidate)?;
    validate_bot(document, "p2", opponent)?;
    validate_reproducibility(document)?;

    let seeds = pointer(document, "/seeds")?
        .as_array()
        .context("/seeds is not an array")?
        .iter()
        .map(|seed| seed.as_u64().context("seed is not an integer"))
        .collect::<Result<Vec<_>>>()?;
    ensure!(seeds == SEEDS, "arena leg must use seeds 7 and 8 in order");
    let runs = pointer(document, "/runs")?
        .as_array()
        .context("/runs is not an array")?;
    ensure!(runs.len() == SEEDS.len());
    for (run, seed) in runs.iter().zip(SEEDS) {
        ensure!(u64_at(run, "/seed")? == seed);
        validate_outcome(pointer(run, "/outcome")?, mode, pairs, candidate, opponent)?;
    }
    validate_pooled_additivity(document)?;
    validate_outcome(
        pointer(document, "/pooled")?,
        mode,
        pairs.checked_mul(2).context("pooled pair count overflow")?,
        candidate,
        opponent,
    )?;
    Ok(())
}

fn load_legs(config: &Config) -> Result<BTreeMap<(String, String), Legs>> {
    let mut grouped = BTreeMap::<(String, String), Legs>::new();
    let mut reproducibility = None;
    for path in &config.inputs {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let mut document: Value =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        let mode = string_at(&document, "/mode")?.to_owned();
        let pairs = match mode.as_str() {
            "rounds" => config.round_pairs,
            "games" => config.game_pairs,
            other => bail!("unsupported arena mode {other:?}"),
        };
        let candidate = bot_spec(&document, "p1")?.to_owned();
        let opponent = bot_spec(&document, "p2")?.to_owned();
        ensure!(
            CANDIDATES.contains(&candidate.as_str()),
            "unexpected candidate {candidate}"
        );
        ensure!(
            OPPONENTS.contains(&opponent.as_str()),
            "unexpected opponent {opponent}"
        );
        validate_document(&document, &mode, pairs, &candidate, &opponent)
            .with_context(|| format!("validate {}", path.display()))?;
        normalize_document_p_values(&mut document)
            .with_context(|| format!("normalize p-values in {}", path.display()))?;
        let current_reproducibility = pointer(&document, "/reproducibility")?;
        if let Some(expected) = &reproducibility {
            validate_same_reproducibility(expected, current_reproducibility)
                .with_context(|| format!("validate {}", path.display()))?;
        } else {
            reproducibility = Some(current_reproducibility.clone());
        }
        let legs = grouped.entry((candidate, opponent)).or_default();
        let slot = if mode == "rounds" {
            &mut legs.rounds
        } else {
            &mut legs.games
        };
        ensure!(slot.is_none(), "duplicate {mode} leg");
        *slot = Some(document);
    }
    ensure!(grouped.len() == CANDIDATES.len() * OPPONENTS.len());
    Ok(grouped)
}

/// Holm's step-down family-wise-error correction, returned in input order.
fn holm_adjust(p_values: &[ExactPValue]) -> Vec<ExactPValue> {
    let mut order = (0..p_values.len()).collect::<Vec<_>>();
    order.sort_by(|&left, &right| p_values[left].ln().total_cmp(&p_values[right].ln()));
    let mut adjusted = vec![ExactPValue::one(); p_values.len()];
    let mut running = None::<ExactPValue>;
    for (rank, index) in order.into_iter().enumerate() {
        let candidate = p_values[index].multiply_clamped(p_values.len() - rank);
        let current = running.map_or(candidate, |previous| {
            if previous.ln() >= candidate.ln() {
                previous
            } else {
                candidate
            }
        });
        running = Some(current);
        adjusted[index] = current;
    }
    adjusted
}

fn direction(difference: f64) -> i8 {
    if difference > 0.0 {
        1
    } else if difference < 0.0 {
        -1
    } else {
        0
    }
}

fn declare_edge(seed_directions: &[i8], adjusted_p: ExactPValue) -> &'static str {
    let significant = adjusted_p.ln() < 0.05_f64.ln();
    if significant && seed_directions == [1, 1] {
        "candidate edge"
    } else if significant && seed_directions == [-1, -1] {
        "opponent edge"
    } else {
        "inconclusive"
    }
}

fn report_verdict(
    evidentiary: bool,
    seed_directions: &[i8],
    adjusted_p: ExactPValue,
) -> &'static str {
    if evidentiary {
        declare_edge(seed_directions, adjusted_p)
    } else {
        "not evaluated (smoke)"
    }
}

fn build_matchups(
    mut grouped: BTreeMap<(String, String), Legs>,
    evidentiary: bool,
) -> Result<Vec<Matchup>> {
    let mut matchups = Vec::with_capacity(6);
    for candidate in CANDIDATES {
        for opponent in OPPONENTS {
            let legs = grouped
                .remove(&(candidate.to_owned(), opponent.to_owned()))
                .with_context(|| format!("missing {candidate} vs {opponent}"))?;
            let rounds = legs.rounds.context("missing rounds leg")?;
            let games = legs.games.context("missing games leg")?;
            let raw_p = exact_p_value_for(pointer(&games, "/pooled")?)?;
            let seed_directions = pointer(&games, "/runs")?
                .as_array()
                .context("game runs is not an array")?
                .iter()
                .map(|run| f64_at(run, "/outcome/comparison/rate_difference").map(direction))
                .collect::<Result<Vec<_>>>()?;
            matchups.push(Matchup {
                candidate: candidate.to_owned(),
                opponent: opponent.to_owned(),
                rounds,
                games,
                raw_p,
                holm_p: ExactPValue::one(),
                seed_directions,
                verdict: "inconclusive",
            });
        }
    }
    let adjusted = holm_adjust(&matchups.iter().map(|item| item.raw_p).collect::<Vec<_>>());
    for (item, adjusted) in matchups.iter_mut().zip(adjusted) {
        item.holm_p = adjusted;
        item.verdict = report_verdict(evidentiary, &item.seed_directions, adjusted);
    }
    ensure!(grouped.is_empty());
    Ok(matchups)
}

fn bot_configurations() -> Result<Value> {
    let mut configurations = serde_json::Map::new();
    for spec in CANDIDATES.into_iter().chain(OPPONENTS) {
        configurations.insert(spec.to_owned(), expected_bot_configuration(spec)?);
    }
    Ok(Value::Object(configurations))
}

fn provenance() -> Value {
    json!({
        "official_eaai_driver": {
            "url": "https://github.com/tneller/gin-rummy-eaai/blob/559c712516e3b0fd6b908864acd141e254d94f39/ginrummy/GinRummyGame.java#L263-L284",
            "commit": EAAI_COMMIT
        },
        "gold": {
            "paper": "https://arxiv.org/html/2607.06854v1",
            "source": "https://github.com/Nikelroid/adversarial-coevolution/blob/3b2f5b7866d27234647c5833497c12ca1a2afde9/agents/gold_standard_agent.py",
            "commit": GOLD_COMMIT,
            "sha256": GOLD_SHA256
        },
        "marjj_v5": {
            "paper": "https://ojs.aaai.org/index.php/AAAI/article/view/17820",
            "source": "https://github.com/aqibahm/MARJJ/blob/5d1f00c1dff5380021785c8146d039a11efcabc3/MARJJ_v5-1.java",
            "commit": MARJJ_COMMIT,
            "sha256": MARJJ_SHA256,
            "official_results": "https://cs.gettysburg.edu/~tneller/games/ginrummy/eaai/gin-rummy-results.pdf"
        },
        "distribution_note": "Independent behavioral adaptations; no upstream agent or GPL framework source is copied or vendored. Both agent repositories lack an explicit license, so benchmark-only placement does not eliminate distribution risk."
    })
}

fn adaptations_and_limits() -> Value {
    json!([
        "These are native host-engine adaptations, not executions of the original agents or reproductions of their published tournaments.",
        "Gold's reported 70–99% results came from a different simplified single-hand RLCard/PettingZoo reward environment; the paper does not claim full-game game-theoretic optimality.",
        "Gold opening offers use its strict draw rule; gin uses the ordinary discard key; Big Gin is replaced by discard-to-gin; canonical local melds and host greedy layoffs are used.",
        "The later public MARJJ_v5 file is a surrogate and is not established as the submitted MARJJ_Player champion binary. Its source constants 18/0.9/7 differ from the paper's 20/0.9/6.",
        "MARJJ uses deterministic canonical C,H,S,D ordering where Java iteration order is unrecoverable, arena-seeded tie selection, host optimized-defender settlement, and host greedy layoffs.",
        "MARJJ round reset is inferred from host callbacks because View has no round identifier. In the pathological case where an opening callback is skipped and the same player receives the exact same ten-card hand in consecutive rounds, stale surrogate history could survive; this limitation is preserved in the measured and conformed adapter.",
        "Defender and layoff semantics can differ from upstream environments, and no EAAI 30-second player timer is enforced.",
        "Mirrored pairs use common random numbers and reverse seats. Because orientation-dependent dead hands retain the dealer, later dealer sequences can diverge."
    ])
}

fn file_sha256(relative: &str) -> Result<String> {
    let output = Command::new("sha256sum")
        .arg(relative)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .with_context(|| format!("hash {relative}"))?;
    ensure!(
        output.status.success(),
        "sha256sum could not hash {relative}"
    );
    String::from_utf8(output.stdout)
        .context("sha256sum output is not UTF-8")?
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .with_context(|| format!("sha256sum returned no digest for {relative}"))
}

fn conformance_source_sha256() -> Result<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = Command::new("sha256sum")
        .args(CONFORMANCE_SOURCE_FILES)
        .current_dir(root)
        .output()
        .context("run sha256sum over conformance sources")?;
    ensure!(
        manifest.status.success(),
        "sha256sum could not read the conformance sources"
    );
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("start conformance manifest digest")?;
    child
        .stdin
        .as_mut()
        .context("conformance digest has no stdin")?
        .write_all(&manifest.stdout)
        .context("hash conformance source manifest")?;
    drop(child.stdin.take());
    let output = child
        .wait_with_output()
        .context("wait for conformance manifest digest")?;
    ensure!(
        output.status.success(),
        "conformance manifest digest failed"
    );
    let digest = String::from_utf8(output.stdout)
        .context("conformance digest is not UTF-8")?
        .split_whitespace()
        .next()
        .context("conformance digest is empty")?
        .to_owned();
    ensure!(
        digest == CONFORMED_NATIVE_SOURCE_SHA256,
        "native adapters or conformance harness changed since the passing receipt: expected {CONFORMED_NATIVE_SOURCE_SHA256}, found {digest}"
    );
    Ok(digest)
}

fn validate_conformance_receipt(receipt: &Value) -> Result<()> {
    ensure!(string_at(receipt, "/schema")? == CONFORMANCE_SCHEMA);
    ensure!(string_at(receipt, "/status")? == "passed");
    ensure!(
        !string_at(receipt, "/checked_on")?.trim().is_empty(),
        "conformance receipt has no check date"
    );
    ensure!(string_at(receipt, "/workflow")? == "scripts/check-strong-conformance.sh");
    ensure!(string_at(receipt, "/gold/commit")? == GOLD_COMMIT);
    ensure!(string_at(receipt, "/gold/source_sha256")? == GOLD_SHA256);
    ensure!(string_at(receipt, "/gold/runtime/python")? == "3.11");
    ensure!(string_at(receipt, "/gold/runtime/pettingzoo")? == "1.24.3");
    ensure!(string_at(receipt, "/gold/runtime/rlcard")? == "1.0.5");
    ensure!(u64_at(receipt, "/gold/agreement/draws")? == 2);
    ensure!(u64_at(receipt, "/gold/agreement/unique_ordinary_discards")? == 128);
    ensure!(u64_at(receipt, "/gold/agreement/unique_non_gin_knocks")? == 32);
    ensure!(u64_at(receipt, "/gold/agreement/gin_category_cases")? == 1);
    ensure!(string_at(receipt, "/marjj_v5/commit")? == MARJJ_COMMIT);
    ensure!(string_at(receipt, "/marjj_v5/source_sha256")? == MARJJ_SHA256);
    ensure!(string_at(receipt, "/marjj_v5/eaai_commit")? == EAAI_COMMIT);
    ensure!(u64_at(receipt, "/marjj_v5/agreement/complete_minimum_meld_sets")? == 65);
    ensure!(u64_at(receipt, "/marjj_v5/agreement/opening_offers")? == 2);
    ensure!(u64_at(receipt, "/marjj_v5/agreement/candidate_component_records")? == 8);
    ensure!(u64_at(receipt, "/marjj_v5/agreement/knock_window_cases")? == 5);
    ensure!(string_at(receipt, "/marjj_v5/agreement/unique_actions")? == "100%");
    let expected_exclusions = json!([
        "iteration order",
        "random ties",
        "floating-point last bits with unchanged minima",
        "documented host-rule adaptations"
    ]);
    ensure!(
        pointer(receipt, "/classified_exclusions")? == &expected_exclusions,
        "conformance exclusions differ from the reviewed receipt"
    );
    Ok(())
}

fn read_validated_conformance_receipt(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("read conformance receipt {}", path.display()))?;
    let receipt: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse conformance receipt {} as JSON", path.display()))?;
    validate_conformance_receipt(&receipt)
        .with_context(|| format!("validate conformance receipt {}", path.display()))?;
    Ok(receipt)
}

fn conformance_preflight_requested() -> Result<bool> {
    let mut args = std::env::args().skip(1);
    let Some(flag) = args.next() else {
        return Ok(false);
    };
    if flag != "--validate-conformance-receipt" {
        return Ok(false);
    }
    let path = PathBuf::from(
        args.next()
            .context("--validate-conformance-receipt needs a path")?,
    );
    ensure!(
        args.next().is_none(),
        "--validate-conformance-receipt accepts exactly one path"
    );
    read_validated_conformance_receipt(&path)?;
    conformance_source_sha256()?;
    Ok(true)
}

fn load_conformance_receipt(config: &Config, arena_source_sha256: &str) -> Result<Value> {
    let Some(path) = &config.conformance_receipt else {
        ensure!(
            config.smoke,
            "publication reports require --conformance-receipt with a passing pinned-source receipt"
        );
        return Ok(json!({
            "status": "not_run",
            "publication_eligible": false,
            "reason": "no explicit conformance receipt was supplied"
        }));
    };
    let receipt = read_validated_conformance_receipt(path)?;
    let native_source_sha256 = conformance_source_sha256()?;
    Ok(json!({
        "status": "passed",
        "publication_eligible": true,
        "receipt_path": path.display().to_string(),
        "arena_source_sha256": arena_source_sha256,
        "native_conformance_source_sha256": native_source_sha256,
        "receipt": receipt
    }))
}

fn build_panel(config: &Config, matchups: &[Matchup], conformance: &Value) -> Result<Value> {
    ensure!(
        config.smoke || string_at(conformance, "/status")? == "passed",
        "publication evidence requires passed upstream conformance"
    );
    let total_runtime = matchups.iter().try_fold(0.0, |total, matchup| {
        Ok::<_, anyhow::Error>(
            total
                + f64_at(&matchup.rounds, "/pooled/elapsed_seconds")?
                + f64_at(&matchup.games, "/pooled/elapsed_seconds")?,
        )
    })?;
    let representative_environment = pointer(&matchups[0].games, "/reproducibility")?.clone();
    let entries = matchups
        .iter()
        .map(|matchup| {
            json!({
                "candidate": matchup.candidate,
                "opponent": matchup.opponent,
                "inference": {
                    "seed_directions": matchup.seed_directions,
                    "pooled_raw_exact_sign_p_value": matchup.raw_p.as_f64(),
                    "pooled_raw_exact_sign_p_value_decimal": matchup.raw_p.decimal(),
                    "pooled_holm_adjusted_p_value": matchup.holm_p.as_f64(),
                    "pooled_holm_adjusted_p_value_decimal": matchup.holm_p.decimal(),
                    "declaration_suppressed": config.smoke,
                    "declaration": matchup.verdict
                },
                "rounds": matchup.rounds,
                "games": matchup.games
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "gin-rummy-strong-opponents/v1",
        "arena_schema": "gin-rummy-arena/v1",
        "evidence_status": if config.smoke { "non_evidentiary_smoke" } else { "publication_panel" },
        "evidentiary": !config.smoke,
        "upstream_conformance": conformance,
        "report_generation": {
            "helper": "examples/strong_report.rs",
            "helper_sha256": file_sha256("examples/strong_report.rs")?,
            "statistics_helper": "examples/support/arena_stats.rs",
            "statistics_helper_sha256": file_sha256("examples/support/arena_stats.rs")?,
            "p_value_normalization": "Exact-sign numeric and scientific-decimal fields were recomputed solely from validated sweep counts in the embedded arena inputs; no rounds or games were rerun. All non-p-value sufficient statistics and measurement metadata are preserved from those inputs."
        },
        "predeclared_design": {
            "candidates": CANDIDATES,
            "opponents": OPPONENTS,
            "seeds": SEEDS,
            "round_pairs_per_matchup_per_seed": config.round_pairs,
            "game_pairs_per_matchup_per_seed": config.game_pairs,
            "total_round_pairs": config.round_pairs * 12,
            "total_rounds": config.round_pairs * 24,
            "total_game_pairs": config.game_pairs * 12,
            "total_games": config.game_pairs * 24,
            "optional_stopping": false,
            "seed_replacement": false,
            "dealer_rotation": "alternate_after_scored_round; dead hands retain dealer",
            "game_score": "raw target-reaching score; no deferred bonuses",
            "primary_test": "two-sided exact sign test over mirrored game-pair sweeps; split pairs are ties",
            "multiplicity": "Holm adjustment over six pooled game-matchup p-values",
            "edge_rule": "both seed estimates have the same nonzero direction and pooled Holm-adjusted p < .05; otherwise inconclusive"
        },
        "bot_configurations": bot_configurations()?,
        "provenance": provenance(),
        "adaptations_and_limitations": adaptations_and_limits(),
        "representative_environment": representative_environment,
        "total_measured_runtime_seconds": total_runtime,
        "matchups": entries
    }))
}

fn outcome(document: &Value, seed: Option<usize>) -> Result<&Value> {
    match seed {
        Some(index) => pointer(document, &format!("/runs/{index}/outcome")),
        None => pointer(document, "/pooled"),
    }
}

fn ci(outcome: &Value) -> Result<(f64, f64)> {
    Ok((
        f64_at(outcome, "/players/0/win_ci95/low")?,
        f64_at(outcome, "/players/0/win_ci95/high")?,
    ))
}

fn percent(value: f64) -> String {
    format!("{:.1}%", 100.0 * value)
}

fn p_value(value: ExactPValue) -> String {
    if value.ln() < 0.001_f64.ln() {
        "<0.001".to_owned()
    } else {
        format!("{:.3}", value.as_f64().expect("p >= .001 fits in f64"))
    }
}

fn game_table(markdown: &mut String, matchups: &[Matchup]) -> Result<()> {
    writeln!(markdown, "## Game results\n")?;
    writeln!(
        markdown,
        "Game win share and its 95% interval use mirrored pairs as clusters. The score margin is candidate minus opponent raw target-reaching score per game. The exact sign test counts only 2–0 pair sweeps; 1–1 splits are ties. Holm adjustment applies to the six pooled rows.\n"
    )?;
    writeln!(
        markdown,
        "| Matchup | Seed | Win share (pair-cluster 95% CI) | Raw score margin/game | Sweeps | Raw p | Holm p | Finding |"
    )?;
    writeln!(markdown, "|---|---:|---:|---:|---:|---:|---:|---|")?;
    for matchup in matchups {
        for (index, seed) in SEEDS.into_iter().enumerate() {
            let result = outcome(&matchup.games, Some(index))?;
            let rate = f64_at(result, "/players/0/win_rate")?;
            let (low, high) = ci(result)?;
            let margin = f64_at(result, "/comparison/point_margin_moments/estimate")?;
            let sweeps = pointer(result, "/comparison/sweeps")?
                .as_array()
                .context("sweeps is not an array")?;
            let raw_p = exact_p_value_for(result)?;
            writeln!(
                markdown,
                "| `{}` vs `{}` | {} | {} ({}–{}) | {:+.2} | {}–{} | {} | — | diagnostic |",
                matchup.candidate,
                matchup.opponent,
                seed,
                percent(rate),
                percent(low),
                percent(high),
                margin,
                sweeps[0].as_u64().context("bad p1 sweep")?,
                sweeps[1].as_u64().context("bad p2 sweep")?,
                p_value(raw_p),
            )?;
        }
        let result = outcome(&matchup.games, None)?;
        let rate = f64_at(result, "/players/0/win_rate")?;
        let (low, high) = ci(result)?;
        let margin = f64_at(result, "/comparison/point_margin_moments/estimate")?;
        let sweeps = pointer(result, "/comparison/sweeps")?
            .as_array()
            .context("sweeps is not an array")?;
        writeln!(
            markdown,
            "| `{}` vs `{}` | pooled | {} ({}–{}) | {:+.2} | {}–{} | {} | {} | **{}** |",
            matchup.candidate,
            matchup.opponent,
            percent(rate),
            percent(low),
            percent(high),
            margin,
            sweeps[0].as_u64().context("bad p1 sweep")?,
            sweeps[1].as_u64().context("bad p2 sweep")?,
            p_value(matchup.raw_p),
            p_value(matchup.holm_p),
            matchup.verdict,
        )?;
    }
    Ok(())
}

fn round_table(markdown: &mut String, matchups: &[Matchup]) -> Result<()> {
    writeln!(markdown, "\n## Single-round diagnostics\n")?;
    writeln!(
        markdown,
        "These pooled single-round results diagnose tactics; they are not the game-strength declaration. Points and paired differential are per individual round. Finish counts are attributed to the bot that won that outcome.\n"
    )?;
    writeln!(
        markdown,
        "| Matchup | Decisive win share (pair-cluster 95% CI) | Points/round | Paired point differential | Dead rate | Candidate K/U/G | Opponent K/U/G |"
    )?;
    writeln!(markdown, "|---|---:|---:|---:|---:|---:|---:|")?;
    for matchup in matchups {
        let result = outcome(&matchup.rounds, None)?;
        let rate = f64_at(result, "/players/0/win_rate")?;
        let (low, high) = ci(result)?;
        let ours = f64_at(result, "/players/0/point_rate_moments/estimate")?;
        let theirs = f64_at(result, "/players/1/point_rate_moments/estimate")?;
        let margin = f64_at(result, "/comparison/point_margin_moments/estimate")?;
        let dead = u64_at(result, "/round_finishes/dead")? as f64;
        let plays = u64_at(result, "/plays")? as f64;
        let finish = |player: usize, kind: &str| {
            u64_at(result, &format!("/players/{player}/finishes/{kind}"))
        };
        writeln!(
            markdown,
            "| `{}` vs `{}` | {} ({}–{}) | {:.2} vs {:.2} | {:+.2} | {} | {}/{}/{} | {}/{}/{} |",
            matchup.candidate,
            matchup.opponent,
            percent(rate),
            percent(low),
            percent(high),
            ours,
            theirs,
            margin,
            percent(dead / plays),
            finish(0, "knock")?,
            finish(0, "undercut")?,
            finish(0, "gin")?,
            finish(1, "knock")?,
            finish(1, "undercut")?,
            finish(1, "gin")?,
        )?;
    }
    Ok(())
}

fn environment_table(
    markdown: &mut String,
    matchups: &[Matchup],
    conformance: &Value,
) -> Result<()> {
    let environment = pointer(&matchups[0].games, "/reproducibility")?;
    writeln!(markdown, "\n## Reproducibility\n")?;
    writeln!(
        markdown,
        "- Measured arena source SHA-256: `{}`",
        string_at(environment, "/source_sha256")?
    )?;
    writeln!(
        markdown,
        "- Cargo.lock SHA-256: `{}`",
        string_at(environment, "/cargo_lock_sha256")?
    )?;
    writeln!(
        markdown,
        "- Git commit: `{}`; dirty worktree: `{}`",
        string_at(environment, "/git_head")?,
        bool_at(environment, "/git_dirty")?
    )?;
    writeln!(
        markdown,
        "- Compiler: `{}`",
        string_at(environment, "/rustc_vv")?.replace('\n', "; ")
    )?;
    writeln!(
        markdown,
        "- Platform: `{}` / `{}`",
        string_at(environment, "/os")?,
        string_at(environment, "/arch")?
    )?;
    let cpu = pointer(environment, "/cpu")?.as_str().unwrap_or("unknown");
    writeln!(
        markdown,
        "- CPU: `{cpu}`; logical threads: `{}`",
        u64_at(environment, "/logical_threads")?
    )?;
    let runtime = matchups.iter().try_fold(0.0, |total, matchup| {
        Ok::<_, anyhow::Error>(
            total
                + f64_at(&matchup.rounds, "/pooled/elapsed_seconds")?
                + f64_at(&matchup.games, "/pooled/elapsed_seconds")?,
        )
    })?;
    writeln!(
        markdown,
        "- Sum of measured leg runtimes: {:.1} seconds",
        runtime
    )?;
    let conformance_status = string_at(conformance, "/status")?;
    if let Some(path) = conformance.get("receipt_path").and_then(Value::as_str) {
        writeln!(
            markdown,
            "- Upstream conformance: `{conformance_status}`; receipt: `{path}`"
        )?;
    } else {
        writeln!(
            markdown,
            "- Upstream conformance: `{conformance_status}` (no receipt supplied)"
        )?;
    }
    writeln!(
        markdown,
        "- Raw machine-readable evidence: [strong-opponents.json](strong-opponents.json)"
    )?;
    writeln!(
        markdown,
        "- Report encoding: exact-sign numeric and scientific-decimal fields were recomputed after measurement solely from the validated sweep counts; no rounds or games were rerun. The raw JSON records the report-helper hashes."
    )?;
    Ok(())
}

fn markdown_report(config: &Config, matchups: &[Matchup], conformance: &Value) -> Result<String> {
    let mut markdown = String::new();
    writeln!(markdown, "# Strong-opponent evaluation\n")?;
    if config.smoke {
        writeln!(
            markdown,
            "**Status: non-evidentiary smoke check. Strength declarations are suppressed.**\n"
        )?;
    }
    writeln!(
        markdown,
        "This fixed panel compares `greedy`, `mc:64`, and `mc:128` with two benchmark-only native adaptations: `gold-paper` and `marjj-v5-surrogate`. These are controlled host-engine comparisons—not executions of the original agents and not reproductions of their published tournaments.\n"
    )?;
    writeln!(
        markdown,
        "Each matchup used seeds 7 and 8, {} mirrored round pairs per seed and {} mirrored game pairs per seed, with no optional stopping, extension, or seed replacement. Games use the corrected EAAI protocol: after a scored hand the dealer flips; after a dead hand the same dealer redeals. Rules are target 100, knock limit 10, gin/undercut bonuses 25, undercut on ties, no Big Gin, and no boxes, game bonus, or shutout bonus. Scores below are the raw totals that reached target.\n",
        config.round_pairs, config.game_pairs
    )?;
    if config.smoke {
        writeln!(
            markdown,
            "The inference calculations below are pipeline diagnostics only. This smoke budget cannot declare an edge; every finding is marked **not evaluated (smoke)**.\n"
        )?;
    } else {
        writeln!(
            markdown,
            "An edge is declared only when both seed estimates point in the same nonzero direction and the pooled exact pair-sweep sign-test p-value remains below .05 after Holm correction across all six matchups. Everything else is **inconclusive**, never “equal.”\n"
        )?;
        writeln!(
            markdown,
            "All three candidates beat `gold-paper` over games (62.0%–67.1% candidate win share) and lost to `marjj-v5-surrogate` (29.2%–34.2%); all six Holm-adjusted p-values were below .001 and both seeds agreed in direction. `mc:128` had the highest observed share against both opponents, but the predeclared tests compare each candidate with its opponent—not candidates with one another.\n"
        )?;
    }
    game_table(&mut markdown, matchups)?;
    round_table(&mut markdown, matchups)?;

    writeln!(markdown, "\n## Provenance and adaptations\n")?;
    writeln!(
        markdown,
        "`gold-paper` follows the fixed heuristic in the [2026 paper](https://arxiv.org/html/2607.06854v1) and [pinned source](https://github.com/Nikelroid/adversarial-coevolution/blob/3b2f5b7866d27234647c5833497c12ca1a2afde9/agents/gold_standard_agent.py) (`88a5ed62638de8c45c0a679c42cd2b05656b93336af9760905d77af04d1e7bca`). Its published 70–99% results came from a different simplified single-hand RLCard/PettingZoo reward environment. Despite the repository label, the paper does not claim game-theoretic optimality for full gin rummy; only meld decomposition is exact. Host-only opening, gin, Big Gin, meld-selection, defender, and layoff behavior is adapted as recorded in the raw JSON.\n"
    )?;
    writeln!(
        markdown,
        "`marjj-v5-surrogate` independently implements the reachable path of the [later public MARJJ_v5 file](https://github.com/aqibahm/MARJJ/blob/5d1f00c1dff5380021785c8146d039a11efcabc3/MARJJ_v5-1.java) (`df6d4db2476ea35ee193258eec12f4925e1ea4d0fb703283fea3b1d4f82b9a4f`). The [official results](https://cs.gettysburg.edu/~tneller/games/ginrummy/eaai/gin-rummy-results.pdf) identify the separately named `MARJJ_Player` as the 2021 winner; they do not establish that this later public file is the submitted binary. The source uses initial future weight 18, discount 0.9, and best-seven selection, while the paper reports 20/0.9/six. Canonical ordering, seeded ties, optimized-defender settlement, and greedy layoffs are host adaptations.\n"
    )?;
    writeln!(
        markdown,
        "Both adapters use host settlement and layoff semantics, which can differ from upstream environments. No EAAI 30-second player timer is enforced. They remain outside the library API and interactive player list. No upstream agent or GPL EAAI framework source is copied or vendored; both agent repositories lack explicit licenses, so benchmark-only placement does not eliminate distribution risk.\n"
    )?;
    writeln!(
        markdown,
        "Because the host `View` has no round identifier, the MARJJ adaptation infers round reset from callbacks. If its opening callback is skipped and the same seat receives the exact same ten-card hand in consecutive rounds, stale surrogate history could theoretically survive. This pathological limitation is retained in the measured, conformed adapter.\n"
    )?;
    writeln!(
        markdown,
        "Mirroring is common-random-number seat reversal, not a guarantee of identical later game histories. Orientation-dependent outcomes—including whether a hand is dead—can make later dealer sequences diverge under dead-hand retention.\n"
    )?;
    writeln!(
        markdown,
        "Commands (release mode, without the `parallel` feature):\n"
    )?;
    writeln!(markdown, "```console")?;
    writeln!(markdown, "scripts/bench-strong.sh --smoke")?;
    writeln!(
        markdown,
        "STRONG_CONFORMANCE_RECEIPT=contrib/strong-conformance/receipt.json \\\n  scripts/bench-strong.sh"
    )?;
    writeln!(markdown, "```")?;
    environment_table(&mut markdown, matchups, conformance)?;
    Ok(markdown)
}

fn write_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

fn main() -> Result<()> {
    if conformance_preflight_requested()? {
        return Ok(());
    }
    let config = parse_args()?;
    let grouped = load_legs(&config)?;
    let matchups = build_matchups(grouped, !config.smoke)?;
    let arena_source_sha256 = string_at(
        pointer(&matchups[0].games, "/reproducibility")?,
        "/source_sha256",
    )?;
    let conformance = load_conformance_receipt(&config, arena_source_sha256)?;
    let panel = build_panel(&config, &matchups, &conformance)?;
    let mut json = serde_json::to_string_pretty(&panel)?;
    json.push('\n');
    let markdown = markdown_report(&config, &matchups, &conformance)?;
    write_file(&config.json_out, &json)?;
    write_file(&config.markdown_out, &markdown)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn additive_outcome(value: u64, signed: i64) -> Value {
        let moments = || {
            json!({
                "clusters": value,
                "numerator": value,
                "denominator": value,
                "numerator_sq": value,
                "numerator_denominator": value,
                "denominator_sq": value
            })
        };
        let player = || {
            json!({
                "wins": value,
                "raw_points": value,
                "finishes": {
                    "knock": value,
                    "undercut": value,
                    "gin": value,
                    "big_gin": value
                },
                "win_rate_moments": moments(),
                "point_rate_moments": moments()
            })
        };
        json!({
            "trials": value,
            "plays": value,
            "decisive": value,
            "failures": value,
            "players": [player(), player()],
            "round_finishes": {
                "knock": value,
                "undercut": value,
                "gin": value,
                "big_gin": value,
                "dead": value
            },
            "comparison": {
                "paired_difference_sum": signed,
                "paired_difference_sq_sum": value,
                "sweeps": [value, value],
                "point_margin_moments": {
                    "clusters": value,
                    "numerator": signed,
                    "denominator": value,
                    "numerator_sq": value,
                    "numerator_denominator": signed,
                    "denominator_sq": value
                }
            }
        })
    }

    #[test]
    fn holm_is_monotone_in_sorted_order_and_restores_input_order() {
        let values = [
            exact_sign_p_value(3, 0).unwrap(),
            exact_sign_p_value(8, 2).unwrap(),
            ExactPValue::one(),
        ];
        let adjusted = holm_adjust(&values);
        let numeric = adjusted
            .into_iter()
            .map(|value| value.as_f64().unwrap())
            .collect::<Vec<_>>();
        for (actual, expected) in numeric.into_iter().zip([0.5, 0.328_125, 1.0]) {
            assert!((actual - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn edge_rule_requires_both_seed_directions_and_adjusted_significance() {
        let significant = exact_sign_p_value(6, 0).unwrap();
        let not_significant = exact_sign_p_value(8, 2).unwrap();
        assert_eq!(declare_edge(&[1, 1], significant), "candidate edge");
        assert_eq!(declare_edge(&[-1, -1], significant), "opponent edge");
        assert_eq!(declare_edge(&[1, -1], significant), "inconclusive");
        assert_eq!(declare_edge(&[1, 1], not_significant), "inconclusive");
        assert_eq!(declare_edge(&[0, 1], significant), "inconclusive");
        assert_eq!(
            report_verdict(false, &[1, 1], significant),
            "not evaluated (smoke)"
        );
    }

    #[test]
    fn p_value_format_covers_unanimous_underflow() {
        assert_eq!(p_value(exact_sign_p_value(4000, 0).unwrap()), "<0.001");
        assert_eq!(p_value(ExactPValue::one()), "1.000");
    }

    #[test]
    fn primary_sweep_test_is_recomputed_and_tampering_is_rejected() {
        let valid = json!({
            "comparison": {
                "sweeps": [8, 2],
                "exact_sign_p_value": 0.109375,
                "exact_sign_p_value_decimal": exact_sign_p_value(8, 2).unwrap().decimal(),
                "primary_test": "exact_sweep_sign",
                "primary_p_value": 0.109375,
                "primary_p_value_decimal": exact_sign_p_value(8, 2).unwrap().decimal()
            }
        });
        validate_primary_fields(&valid).expect("valid exact sign fields");

        let mut bad_p = valid.clone();
        bad_p["comparison"]["primary_p_value"] = json!(0.01);
        assert!(validate_primary_fields(&bad_p).is_err());

        let mut bad_exact = valid.clone();
        bad_exact["comparison"]["exact_sign_p_value"] = json!(0.01);
        assert!(validate_primary_fields(&bad_exact).is_err());

        let mut bad_decimal = valid.clone();
        bad_decimal["comparison"]["primary_p_value_decimal"] = json!("1e-99");
        assert!(validate_primary_fields(&bad_decimal).is_err());

        let mut bad_name = valid;
        bad_name["comparison"]["primary_test"] = json!("normal_z");
        assert!(validate_primary_fields(&bad_name).is_err());
    }

    #[test]
    fn legacy_underflow_is_accepted_only_for_migration_and_normalized() {
        let mut legacy = json!({
            "comparison": {
                "sweeps": [4000, 0],
                "exact_sign_p_value": 0.0,
                "primary_test": "exact_sweep_sign",
                "primary_p_value": 0.0
            }
        });
        validate_primary_fields(&legacy).expect("legacy arena underflow can be migrated");
        normalize_outcome_p_values(&mut legacy).expect("normalize legacy p-values");
        assert!(legacy["comparison"]["exact_sign_p_value"].is_null());
        assert!(legacy["comparison"]["primary_p_value"].is_null());
        assert_eq!(
            legacy["comparison"]["exact_sign_p_value_decimal"],
            exact_sign_p_value(4000, 0).unwrap().decimal()
        );
        validate_primary_fields(&legacy).expect("normalized fields validate");
    }

    #[test]
    fn game_sweeps_must_match_wins_splits_and_paired_moments() {
        let valid = json!({
            "comparison": {
                "sweeps": [3, 2],
                "paired_difference_sum": 2,
                "paired_difference_sq_sum": 20,
                "exact_sign_p_value": 1.0,
                "primary_test": "exact_sweep_sign",
                "primary_p_value": 1.0
            }
        });
        validate_primary_fields(&valid).expect("valid primary fields");
        validate_sweep_consistency(&valid, "games", 10, [11, 9])
            .expect("five sweeps and five splits exhaust ten game pairs");

        // Recomputing p from coordinated but false sweep counts is not enough:
        // the wins and paired moments must independently corroborate them.
        let mut bad_sweeps = valid.clone();
        bad_sweeps["comparison"]["sweeps"] = json!([4, 1]);
        bad_sweeps["comparison"]["exact_sign_p_value"] = json!(0.375);
        bad_sweeps["comparison"]["primary_p_value"] = json!(0.375);
        validate_primary_fields(&bad_sweeps).expect("coordinated p matches false sweeps");
        assert!(validate_sweep_consistency(&bad_sweeps, "games", 10, [11, 9]).is_err());

        let mut bad_square_sum = valid;
        bad_square_sum["comparison"]["paired_difference_sq_sum"] = json!(16);
        assert!(validate_sweep_consistency(&bad_square_sum, "games", 10, [11, 9]).is_err());
    }

    #[test]
    fn pooled_sufficient_statistics_must_be_additive() {
        let valid = json!({
            "runs": [
                {"outcome": additive_outcome(1, -1)},
                {"outcome": additive_outcome(3, 2)}
            ],
            "pooled": additive_outcome(4, 1)
        });
        validate_pooled_additivity(&valid).expect("valid additive pool");

        let mut bad_moment = valid.clone();
        bad_moment["pooled"]["players"][0]["win_rate_moments"]["numerator_sq"] = json!(5);
        assert!(validate_pooled_additivity(&bad_moment).is_err());

        let mut bad_signed = valid;
        bad_signed["pooled"]["comparison"]["point_margin_moments"]["numerator_denominator"] =
            json!(0);
        assert!(validate_pooled_additivity(&bad_signed).is_err());
    }

    #[test]
    fn pair_cluster_estimate_and_zero_width_ci_are_verified() {
        let valid = json!({
            "moments": {
                "clusters": 8,
                "numerator": 16,
                "denominator": 16,
                "numerator_sq": 32,
                "numerator_denominator": 32,
                "denominator_sq": 32,
                "estimate": 1.0,
                "cluster_ci95": {"low": 1.0, "high": 1.0}
            }
        });
        validate_ratio_moments(&valid, "/moments", 8, 16, 16, true)
            .expect("unanimous clusters have a valid zero-width interval");

        let mut bad_estimate = valid.clone();
        bad_estimate["moments"]["estimate"] = json!(0.99);
        assert!(validate_ratio_moments(&bad_estimate, "/moments", 8, 16, 16, true).is_err());

        let mut bad_ci = valid.clone();
        bad_ci["moments"]["cluster_ci95"]["low"] = json!(0.9);
        assert!(validate_ratio_moments(&bad_ci, "/moments", 8, 16, 16, true).is_err());

        let player = json!({
            "win_ci95": {"method": "pair_cluster_normal", "low": 1.0, "high": 1.0}
        });
        validate_win_interval(&player, &valid, "/moments").expect("matching pair CI");
        let mut bad_method = player;
        bad_method["win_ci95"]["method"] = json!("wilson");
        assert!(validate_win_interval(&bad_method, &valid, "/moments").is_err());
    }

    #[test]
    fn raw_configuration_and_provenance_tampering_is_rejected() {
        let marjj = expected_bot_configuration("marjj-v5-surrogate")
            .expect("predeclared MARJJ configuration");
        assert_eq!(marjj["initial_future_weight"], 18);
        assert!(marjj.get("estimated_turns").is_none());
        let valid_bot = json!({
            "bots": {
                "p2": {"spec": "marjj-v5-surrogate", "configuration": marjj}
            }
        });
        validate_bot(&valid_bot, "p2", "marjj-v5-surrogate").expect("exact raw configuration");
        let mut bad_bot = valid_bot;
        bad_bot["bots"]["p2"]["configuration"]["future_cards"] = json!(6);
        assert!(validate_bot(&bad_bot, "p2", "marjj-v5-surrogate").is_err());

        let expected = json!({"source_sha256": "aaa", "git_head": "bbb"});
        validate_same_reproducibility(&expected, &expected).expect("identical provenance");
        let actual = json!({"source_sha256": "ccc", "git_head": "bbb"});
        assert!(validate_same_reproducibility(&expected, &actual).is_err());
    }

    #[test]
    fn publication_receipt_is_pinned_and_native_sources_are_bound() {
        let receipt: Value =
            serde_json::from_str(include_str!("../contrib/strong-conformance/receipt.json"))
                .expect("checked receipt is JSON");
        validate_conformance_receipt(&receipt).expect("checked receipt matches all pins");
        assert_eq!(
            conformance_source_sha256().expect("hash conformance sources"),
            CONFORMED_NATIVE_SOURCE_SHA256
        );

        let mut bad_status = receipt.clone();
        bad_status["status"] = json!("failed");
        assert!(validate_conformance_receipt(&bad_status).is_err());
        let mut bad_pin = receipt.clone();
        bad_pin["gold"]["source_sha256"] = json!("unreviewed");
        assert!(validate_conformance_receipt(&bad_pin).is_err());
        let mut bad_corpus = receipt;
        bad_corpus["marjj_v5"]["agreement"]["complete_minimum_meld_sets"] = json!(64);
        assert!(validate_conformance_receipt(&bad_corpus).is_err());
    }

    #[test]
    fn publication_refuses_missing_receipt_but_smoke_records_not_run() {
        let config = |smoke| Config {
            inputs: Vec::new(),
            json_out: PathBuf::from("unused.json"),
            markdown_out: PathBuf::from("unused.md"),
            round_pairs: if smoke {
                SMOKE_PAIRS
            } else {
                PUBLICATION_ROUND_PAIRS
            },
            game_pairs: if smoke {
                SMOKE_PAIRS
            } else {
                PUBLICATION_GAME_PAIRS
            },
            smoke,
            conformance_receipt: None,
        };
        assert!(load_conformance_receipt(&config(false), "arena-digest").is_err());
        let smoke = load_conformance_receipt(&config(true), "arena-digest")
            .expect("smoke may omit conformance");
        assert_eq!(smoke["status"], "not_run");
        assert_eq!(smoke["publication_eligible"], false);
    }
}
