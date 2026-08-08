#!/usr/bin/env bash
# Run the predeclared strong-opponent panel and generate its evidence files.
#
#   scripts/bench-strong.sh --smoke  # 20 pairs for every leg; no repo writes
#   STRONG_CONFORMANCE_RECEIPT=contrib/strong-conformance/receipt.json \
#     scripts/bench-strong.sh        # fixed publication budget; writes docs/
#
# The arena parallelizes trials.  Do not add Cargo's `parallel` feature:
# nested Monte Carlo parallelism would contend for the same rayon pool.
set -euo pipefail
cd "$(dirname "$0")/.."

case "${1:-}" in
    "")
        round_pairs=4000
        game_pairs=3000
        publish=1
        ;;
    --smoke)
        round_pairs=20
        game_pairs=20
        publish=0
        ;;
    *)
        echo "usage: scripts/bench-strong.sh [--smoke]" >&2
        exit 2
        ;;
esac

if [[ "$publish" == 1 && -z "${STRONG_CONFORMANCE_RECEIPT:-}" ]]; then
    echo "publication requires STRONG_CONFORMANCE_RECEIPT=path/to/passing-receipt.json" >&2
    exit 2
fi
if [[ -n "${STRONG_CONFORMANCE_RECEIPT:-}" && ! -f "$STRONG_CONFORMANCE_RECEIPT" ]]; then
    echo "conformance receipt does not exist: $STRONG_CONFORMANCE_RECEIPT" >&2
    exit 2
fi

work=$(mktemp -d)
cleanup() {
    if [[ -n "${work:-}" && -d "$work" ]]; then
        rm -rf -- "$work"
    fi
}
trap cleanup EXIT

candidates=(greedy mc:64 mc:128)
opponents=(gold-paper marjj-v5-surrogate)
legs=()

run_leg() {
    local mode=$1 pairs=$2 candidate=$3 opponent=$4
    local safe_candidate=${candidate//:/_}
    local output="$work/${safe_candidate}__${opponent}__${mode}.json"
    echo "+ $candidate vs $opponent: $pairs mirrored $mode pairs per seed" >&2
    "$arena_bin" \
        "--$mode" "$pairs" \
        --p1 "$candidate" \
        --p2 "$opponent" \
        --rules eaai \
        --alternate-dealer \
        --seeds 7,8 \
        --format json >"$output"
    legs+=("$output")
}

# Build both helpers before starting the clocked legs, then run the smoke or
# fixed panel without optional stopping, extension, or seed replacement.
cargo_target_dir=${CARGO_TARGET_DIR:-target}
cargo build --quiet --release --target-dir "$cargo_target_dir" \
    --example arena --example strong_report
arena_bin="$cargo_target_dir/release/examples/arena"
report_bin="$cargo_target_dir/release/examples/strong_report"
[[ -x "$arena_bin" && -x "$report_bin" ]]
if [[ -n "${STRONG_CONFORMANCE_RECEIPT:-}" ]]; then
    "$report_bin" --validate-conformance-receipt "$STRONG_CONFORMANCE_RECEIPT"
fi
for candidate in "${candidates[@]}"; do
    for opponent in "${opponents[@]}"; do
        run_leg rounds "$round_pairs" "$candidate" "$opponent"
        run_leg games "$game_pairs" "$candidate" "$opponent"
    done
done

if [[ "$publish" == 1 ]]; then
    json_out=docs/strong-opponents.json
    markdown_out=docs/strong-opponents.md
else
    json_out="$work/strong-opponents-smoke.json"
    markdown_out="$work/strong-opponents-smoke.md"
fi

report_flags=()
if [[ "$publish" == 0 ]]; then
    report_flags+=(--smoke)
fi
if [[ -n "${STRONG_CONFORMANCE_RECEIPT:-}" ]]; then
    report_flags+=(--conformance-receipt "$STRONG_CONFORMANCE_RECEIPT")
fi

"$report_bin" \
    --round-pairs "$round_pairs" \
    --game-pairs "$game_pairs" \
    --json-out "$json_out" \
    --markdown-out "$markdown_out" \
    "${report_flags[@]}" \
    "${legs[@]}"

if [[ "$publish" == 1 ]]; then
    echo "wrote $json_out and $markdown_out" >&2
else
    echo "20-pair smoke passed; generated report follows" >&2
    sed -n '1,80p' "$markdown_out"
fi
