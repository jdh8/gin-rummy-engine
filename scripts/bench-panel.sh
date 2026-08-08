#!/bin/bash
# Regenerate the corrected EAAI baseline panel from arena JSON.
#
#   scripts/bench-panel.sh > panel.md 2> panel.log
#
# Markdown is written to stdout.  Commands and the raw arena JSON evidence
# are written to stderr.  Every trial is a mirrored pair under the exact
# EAAI rules and scored-hand-only dealer rotation.
#
# Full fixed panel:
#   - greedy/mc:64: 4000 round pairs at seed 7, then 3000 game pairs at
#     each of seeds 7 and 8;
#   - mc:128: 4000 round pairs, then 2000 game pairs at each game seed;
#   - mc:64 vs greedy: 3000 game pairs at each game seed.
#
# Shrink it for a smoke test without changing the panel shape:
#   ROUND_PAIRS=20 GAME_PAIRS=20 GAME_PAIRS_128=20 scripts/bench-panel.sh
#
# Never add --features parallel: in-decision parallelism would fight the
# arena's trial-level fan-out for the same rayon pool.
set -euo pipefail
cd "$(dirname "$0")/.."

ROUND_PAIRS=${ROUND_PAIRS:-4000}
GAME_PAIRS=${GAME_PAIRS:-3000}
GAME_PAIRS_128=${GAME_PAIRS_128:-2000}
ROUND_SEED=${ROUND_SEED:-7}
SEEDS=${SEEDS:-"7 8"}

# Preserve the historical whitespace-separated SEEDS override while also
# accepting the comma-separated spelling used by the arena CLI.
seed_text=${SEEDS//,/ }
read -r -a seed_values <<<"$seed_text"
if ((${#seed_values[@]} == 0)); then
    echo "SEEDS must contain at least one seed" >&2
    exit 2
fi
for seed in "${seed_values[@]}"; do
    if [[ ! $seed =~ ^[0-9]+$ ]]; then
        echo "invalid seed: $seed" >&2
        exit 2
    fi
done
printf -v seed_csv '%s,' "${seed_values[@]}"
seed_csv=${seed_csv%,}

scratch=$(mktemp -d "${TMPDIR:-/tmp}/gin-rummy-baseline.XXXXXXXX")
trap 'rm -r -- "$scratch"' EXIT

echo "+ cargo build --quiet --release --example arena --example baseline_report" >&2
cargo build --quiet --release --example arena --example baseline_report
target_dir=${CARGO_TARGET_DIR:-target}
arena_bin="$target_dir/release/examples/arena"
report_bin="$target_dir/release/examples/baseline_report"

# Run one arena leg and retain its machine-readable evidence for the Rust
# reporter.  Echoing the JSON to stderr preserves the old panel.log audit
# trail without parsing the text presentation at any point.
arena_json() {
    local destination=$1
    shift
    echo "+ $arena_bin $* --paired --format json" >&2
    "$arena_bin" "$@" --paired --format json >"$destination"
    while IFS= read -r line; do
        printf '%s\n' "$line" >&2
    done <"$destination"
}

inputs=()
for bot in greedy mc:64 mc:128; do
    tag=${bot//:/-}
    rounds_path="$scratch/$tag-rounds.json"
    games_path="$scratch/$tag-games.json"
    if [[ $bot == mc:128 ]]; then
        game_pairs=$GAME_PAIRS_128
    else
        game_pairs=$GAME_PAIRS
    fi

    arena_json "$rounds_path" \
        --rounds "$ROUND_PAIRS" --p1 "$bot" --p2 eaai \
        --rules eaai --alternate-dealer --seed "$ROUND_SEED"
    arena_json "$games_path" \
        --games "$game_pairs" --p1 "$bot" --p2 eaai \
        --rules eaai --alternate-dealer --seeds "$seed_csv"
    inputs+=("$rounds_path" "$games_path")
done

head_to_head_path="$scratch/mc-64-vs-greedy-games.json"
arena_json "$head_to_head_path" \
    --games "$GAME_PAIRS" --p1 mc:64 --p2 greedy \
    --rules eaai --alternate-dealer --seeds "$seed_csv"
inputs+=("$head_to_head_path")

stamp=$(git rev-parse --short HEAD)
if [[ -n $(git status --porcelain) ]]; then
    stamp="$stamp-dirty"
fi

echo "+ $report_bin ${inputs[*]}" >&2
"$report_bin" \
    --stamp "$stamp" \
    --round-pairs "$ROUND_PAIRS" \
    --game-pairs "$GAME_PAIRS" \
    --game-pairs-128 "$GAME_PAIRS_128" \
    --round-seed "$ROUND_SEED" \
    --seeds "$seed_csv" \
    "${inputs[@]}"
