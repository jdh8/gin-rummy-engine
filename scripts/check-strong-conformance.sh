#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/check-strong-conformance.sh [OPTIONS]

Run opt-in, network-free checks against user-supplied upstream sources.

  --gold-root PATH    adversarial-coevolution checkout/archive root
  --marjj-root PATH   MARJJ checkout/archive root
  --eaai-root PATH    gin-rummy-eaai checkout/archive root (required for MARJJ)
  --python PATH       Python 3.11 interpreter (default: python3.11)
  -h, --help          show this help

At least one of --gold-root or --marjj-root is required.  The checker never
downloads dependencies or source code.
EOF
}

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd -P)
gold_root=
marjj_root=
eaai_root=
python_bin=python3.11

while (($#)); do
  case "$1" in
    --gold-root)
      [[ $# -ge 2 ]] || { echo "missing value for --gold-root" >&2; exit 2; }
      gold_root=$2
      shift 2
      ;;
    --marjj-root)
      [[ $# -ge 2 ]] || { echo "missing value for --marjj-root" >&2; exit 2; }
      marjj_root=$2
      shift 2
      ;;
    --eaai-root)
      [[ $# -ge 2 ]] || { echo "missing value for --eaai-root" >&2; exit 2; }
      eaai_root=$2
      shift 2
      ;;
    --python)
      [[ $# -ge 2 ]] || { echo "missing value for --python" >&2; exit 2; }
      python_bin=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$gold_root" && -z "$marjj_root" ]]; then
  echo "supply --gold-root, --marjj-root, or both" >&2
  exit 2
fi
if [[ -n "$marjj_root" && -z "$eaai_root" ]]; then
  echo "--eaai-root is required with --marjj-root" >&2
  exit 2
fi

scratch=$(mktemp -d "${TMPDIR:-/tmp}/gin-rummy-strong-conformance.XXXXXXXX")
trap 'rm -r -- "$scratch"' EXIT

digest() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -- "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -- "$1" | awk '{print $1}'
  else
    echo "sha256sum or shasum is required" >&2
    exit 2
  fi
}

require_digest() {
  local file=$1
  local expected=$2
  [[ -f "$file" ]] || { echo "missing source file: $file" >&2; exit 2; }
  local actual
  actual=$(digest "$file")
  if [[ "$actual" != "$expected" ]]; then
    echo "source digest mismatch: $file" >&2
    echo "  expected $expected" >&2
    echo "  actual   $actual" >&2
    exit 2
  fi
  echo "verified sha256 $actual  $file"
}

require_commit_when_checkout() {
  local root=$1
  local expected=$2
  local label=$3
  local checkout_root=
  if checkout_root=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null) \
      && [[ "$checkout_root" == "$root" ]]; then
    local actual
    actual=$(git -C "$root" rev-parse HEAD)
    if [[ "$actual" != "$expected" ]]; then
      echo "$label commit mismatch: expected $expected, found $actual" >&2
      exit 2
    fi
    echo "verified commit $actual  $label"
  else
    echo "classified: $label is an archive/source tree; exact file hashes substitute for commit metadata"
  fi
}

if [[ -n "$gold_root" ]]; then
  gold_root=$(CDPATH= cd -- "$gold_root" && pwd -P)
  require_commit_when_checkout \
    "$gold_root" \
    3b2f5b7866d27234647c5833497c12ca1a2afde9 \
    Gold
  require_digest \
    "$gold_root/agents/gold_standard_agent.py" \
    88a5ed62638de8c45c0a679c42cd2b05656b93336af9760905d77af04d1e7bca
  require_digest \
    "$gold_root/agents/agent.py" \
    ad41ce7a9c0fdded0703cdf7162639bc26fc450ca8dfb20b09b0660455d9d340
  require_digest \
    "$gold_root/agents/__init__.py" \
    3febf1777f68550ff6b9e3306e4a78055af83a88b9cf96b69e83dd97da688529
  gold_stage="$scratch/gold"
  mkdir -p "$gold_stage/agents"
  cp -- "$gold_root/agents/gold_standard_agent.py" "$gold_stage/agents/gold_standard_agent.py"
  cp -- "$gold_root/agents/agent.py" "$gold_stage/agents/agent.py"
  cp -- "$gold_root/agents/__init__.py" "$gold_stage/agents/__init__.py"
  "$python_bin" \
    "$repo_root/contrib/strong-conformance/gold_probe.py" \
    --root "$gold_stage" --check
  (
    cd "$repo_root"
    GOLD_UPSTREAM_ROOT="$gold_stage" GOLD_PYTHON="$python_bin" \
      cargo test --locked --offline --features rand --test strong_conformance \
      gold_upstream_unique_decisions -- \
      --ignored --exact --nocapture
  )
fi

if [[ -n "$marjj_root" ]]; then
  marjj_root=$(CDPATH= cd -- "$marjj_root" && pwd -P)
  eaai_root=$(CDPATH= cd -- "$eaai_root" && pwd -P)
  require_commit_when_checkout \
    "$marjj_root" \
    5d1f00c1dff5380021785c8146d039a11efcabc3 \
    MARJJ
  require_digest \
    "$marjj_root/MARJJ_v5-1.java" \
    df6d4db2476ea35ee193258eec12f4925e1ea4d0fb703283fea3b1d4f82b9a4f
  require_commit_when_checkout \
    "$eaai_root" \
    559c712516e3b0fd6b908864acd141e254d94f39 \
    EAAI
  require_digest \
    "$eaai_root/ginrummy/Card.java" \
    6dd77c04f724d8ef1f9803ed4cccb0e92ce04a40d7f044d77294d7d847c019d4
  require_digest \
    "$eaai_root/ginrummy/GinRummyPlayer.java" \
    eb9a62ee295d5291a3c6c1642d79027f443bd32a226fbde7dec665951a13340c
  require_digest \
    "$eaai_root/ginrummy/GinRummyUtil.java" \
    833c4029446bc56ac376bb4c2ca178a31ae999750180adbeff906b2de804dbc9

  command -v javac >/dev/null 2>&1 || { echo "javac is required" >&2; exit 2; }
  command -v java >/dev/null 2>&1 || { echo "java is required" >&2; exit 2; }
  stage="$scratch/marjj"
  mkdir -p "$stage/src/ginrummy" "$stage/classes"
  cp -- "$eaai_root/ginrummy/Card.java" "$stage/src/ginrummy/Card.java"
  cp -- "$eaai_root/ginrummy/GinRummyPlayer.java" "$stage/src/ginrummy/GinRummyPlayer.java"
  cp -- "$eaai_root/ginrummy/GinRummyUtil.java" "$stage/src/ginrummy/GinRummyUtil.java"
  awk 'BEGIN { print "package ginrummy;"; print "" } { print }' \
    "$marjj_root/MARJJ_v5-1.java" > "$stage/src/ginrummy/MARJJ_v5.java"
  cp -- "$repo_root/contrib/strong-conformance/MarjjTrace.java" \
    "$stage/src/ginrummy/MarjjTrace.java"
  javac -d "$stage/classes" "$stage"/src/ginrummy/*.java
  java -cp "$stage/classes" ginrummy.MarjjTrace --self-check
  (
    cd "$repo_root"
    MARJJ_TRACE_CLASSPATH="$stage/classes" \
      cargo test --locked --offline --features rand --test strong_conformance \
      marjj_upstream_unique_decisions -- \
      --ignored --exact --nocapture
  )
fi

echo "conformance checks passed"
echo "classified exclusions: host-only phases, source ordering, random ties, and unchanged-minimum floating-point tails"
