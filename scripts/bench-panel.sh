#!/bin/bash
# Regenerate the benchmark panel in README.md's "Benchmarks" section.
#
#   scripts/bench-panel.sh > panel.md      # table on stdout, arena log on stderr
#
# Every bot plays the EAAI-2021 challenge baseline under its rules and
# dealer protocol, in mirrored pairs, at pinned seeds: rounds at seed 7,
# games pooled over seeds 7 and 8.  Seeds and counts are fixed so the
# published numbers are reproducible; the arena is deterministic in the
# seed, so a rerun at the same commit must print the same table.
#
# Budget ~1.5 hours (the Monte Carlo game legs dominate).  Shrink it for
# a smoke test: GAME_PAIRS=20 GAME_PAIRS_128=20 ROUND_PAIRS=20.
#
# Never add --features parallel: in-decision parallelism would fight the
# arena's trial-level fan-out for the same rayon pool.
set -euo pipefail
cd "$(dirname "$0")/.."

ROUND_PAIRS=${ROUND_PAIRS:-4000}
GAME_PAIRS=${GAME_PAIRS:-3000}
GAME_PAIRS_128=${GAME_PAIRS_128:-2000}   # halved: mc:128 costs twice as much
ROUND_SEED=${ROUND_SEED:-7}
SEEDS=${SEEDS:-"7 8"}

# Run the arena, echoing the command and its output to stderr as a log
# while returning the output for parsing.  Not `tee /dev/stderr`: when
# stderr is a file, tee reopens and truncates it, erasing the log.
arena() {
    local out
    echo "+ arena $*" >&2
    out=$(cargo run --quiet --release --example arena -- "$@")
    printf '%s\n' "$out" >&2
    printf '%s\n' "$out"
}

# "p1=mc:64: 3583 game wins / 6000 (59.7%, ...)"  -> "3583 6000"
# "p1=mc:64: 3152 wins / 8000 decisive (39.4%, ...)" -> "3152 8000"
wins_of() {
    sed -n "s|^$1=.*: \([0-9][0-9]*\) \(game \)\{0,1\}wins / \([0-9][0-9]*\).*|\1 \3|p"
}

# The trailing "8.92 points/round" of a rounds line
points_of() {
    sed -n "s|^$1=.*), \([0-9.][0-9.]*\) points/round|\1|p"
}

# The 95% Wilson score interval, as README prints it.  Mirrors `wilson`
# in examples/arena.rs, which the pooled game legs cannot reuse: they sum
# two seeds' counts before the interval is taken.
wilson() {
    awk -v k="${1:-0}" -v n="${2:-0}" 'BEGIN {
        if (n == 0) { print "n/a"; exit }
        z = 1.96; p = k / n; denom = 1 + z * z / n
        center = (p + z * z / (2 * n)) / denom
        half = z * sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / denom
        printf "%.1f%% (%.1f–%.1f)\n", 100 * p, 100 * (center - half), 100 * (center + half)
    }'
}

stamp=$(git rev-parse --short HEAD)
if [ -n "$(git status --porcelain)" ]; then
    stamp="$stamp-dirty"
fi

echo "<!-- scripts/bench-panel.sh at $stamp:" \
     "ROUND_PAIRS=$ROUND_PAIRS GAME_PAIRS=$GAME_PAIRS" \
     "GAME_PAIRS_128=$GAME_PAIRS_128 ROUND_SEED=$ROUND_SEED SEEDS=\"$SEEDS\" -->"
echo
echo "| Bot vs baseline | Rounds won        | Points/round | Games won         |"
echo "|-----------------|-------------------|--------------|-------------------|"

for bot in greedy mc:64 mc:128; do
    if [ "$bot" = mc:128 ]; then pairs=$GAME_PAIRS_128; else pairs=$GAME_PAIRS; fi

    rounds=$(arena --rounds "$ROUND_PAIRS" --p1 "$bot" --p2 eaai --rules eaai \
                   --seed "$ROUND_SEED")
    read -r won decisive <<<"$(wins_of p1 <<<"$rounds")"
    ours=$(points_of p1 <<<"$rounds")
    theirs=$(points_of p2 <<<"$rounds")

    games_won=0 games_played=0
    for seed in $SEEDS; do
        games=$(arena --games "$pairs" --p1 "$bot" --p2 eaai --rules eaai \
                      --alternate-dealer --seed "$seed")
        read -r w n <<<"$(wins_of p1 <<<"$games")"
        games_won=$((games_won + w)) games_played=$((games_played + n))
    done

    printf '| %-15s | %-17s | %-12s | %-17s |\n' \
        "\`$bot\`" "$(wilson "$won" "$decisive")" "$ours vs $theirs" \
        "$(wilson "$games_won" "$games_played")"
done

# The head-to-head cross-check: exploiting the weak baseline is not
# transitive with strength, so README quotes this one too.
won=0 played=0 lines=
for seed in $SEEDS; do
    games=$(arena --games "$GAME_PAIRS" --p1 mc:64 --p2 greedy --rules eaai \
                  --alternate-dealer --seed "$seed")
    read -r w n <<<"$(wins_of p1 <<<"$games")"
    won=$((won + w)) played=$((played + n))
    lines="$lines seed $seed:$(grep ' vs p2=' <<<"$games" | sed 's/.*pp,//');"
done
echo
echo "head-to-head, \`mc:64\` vs \`greedy\`: $(wilson "$won" "$played")" \
     "of $played paired games —${lines%;}"
