#!/usr/bin/env bash
#
# Import the commands in your zsh history that hoard does not already know.
#
#   ./scripts/import-zsh-history.sh                  # write new entries
#   ./scripts/import-zsh-history.sh --dry-run        # report only, changes nothing
#   ./scripts/import-zsh-history.sh --json           # one compact JSON document
#   ./scripts/import-zsh-history.sh --histfile ~/h -n mine --limit 200
#
# Duplicate detection is hoard's job: `history import --all` skips every command
# whose text is already in the trove, so re-running this is always safe. Secrets
# are redacted before anything is stored (see `--trufflehog` below).
set -euo pipefail

# Run `hoard history scan` instead of importing (writes nothing).
dry_run=0
# Ask hoard for `--json --compact` and mirror its error shape ourselves.
json=0
histfile=""
namespace=""
limit=""
trufflehog=""

usage() {
  cat <<'EOF'
Import commands from zsh history that are not in the trove yet.

Usage: import-zsh-history.sh [OPTIONS]

Options:
  --dry-run             Report what would be imported; write nothing
  --histfile <PATH>     History file to read (default: $HISTFILE, else ~/.zsh_history)
  -n, --namespace <NS>  Store every imported entry in this namespace
  --limit <N>           Consider at most N candidates
  --trufflehog <MODE>   Secret scanning: auto (default), off, or a binary path
  -j, --json            Emit one compact JSON document (implies no header text)
  -h, --help            Print this message

The hoard binary is taken from $HOARD_BIN, then a build in ./target/{release,debug},
  then $PATH.
EOF
}

die() {
  local error_type=$1 message=$2 code=${3:-1}
  if [[ $json == 1 ]]; then
    printf '{"ok":false,"error_type":"%s","message":"%s"}\n' "$error_type" "$message"
  else
    printf 'error: %s\n' "$message" >&2
  fi
  exit "$code"
}

# Expand a leading ~ so `--histfile ~/x` works when the shell did not.
expand_home() {
  local path=$1
  if [[ $path == \~ ]]; then
    path=$HOME
  elif [[ $path == \~/* ]]; then
    path=$HOME/${path:2}
  fi
  printf '%s' "$path"
}

while (($#)); do
  case $1 in
    --dry-run) dry_run=1 ;;
    --histfile)
      (($# >= 2)) || die ParseError "--histfile needs a path" 1
      histfile=$(expand_home "$2")
      shift
      ;;
    --histfile=*)
      histfile=$(expand_home "${1#--histfile=}")
      ;;
    -n | --namespace)
      (($# >= 2)) || die ParseError "$1 needs a value" 1
      namespace=$2
      shift
      ;;
    --namespace=*)
      namespace=${1#--namespace=}
      ;;
    --limit)
      (($# >= 2)) || die ParseError "--limit needs a number" 1
      limit=$2
      shift
      ;;
    --limit=*)
      limit=${1#--limit=}
      ;;
    --trufflehog)
      (($# >= 2)) || die ParseError "--trufflehog needs a value" 1
      trufflehog=$2
      shift
      ;;
    --trufflehog=*)
      trufflehog=${1#--trufflehog=}
      ;;
    -j | --json) json=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die ParseError "unknown argument: $1 (try --help)" 1 ;;
  esac
  shift
done

find_hoard() {
  if [[ -n ${HOARD_BIN:-} ]]; then
    printf '%s' "$HOARD_BIN"
    return 0
  fi
  # A build from this checkout wins over $PATH: when you run this from the repo
  # you want the code you just wrote, not a cargo-installed copy that predates
  # `--compact`. Set HOARD_BIN to override.
  local root candidate
  root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
  for candidate in "$root/target/release/hoard" "$root/target/debug/hoard"; do
    if [[ -x $candidate ]]; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  if command -v hoard >/dev/null 2>&1; then
    command -v hoard
    return 0
  fi
  return 1
}

hoard=$(find_hoard) ||
  die CommandNotFound "no hoard binary found (set \$HOARD_BIN or build with cargo)" 1

# `hoard history` resolves the same defaults; resolving here lets us fail with a
# message that names the file instead of hoard's per-source "not found" line.
if [[ -z $histfile ]]; then
  histfile=${HISTFILE:-$HOME/.zsh_history}
  histfile=$(expand_home "$histfile")
fi
[[ -f $histfile ]] ||
  die HistoryNotFound "no zsh history at $histfile (pass --histfile)" 1

args=(history)
if [[ $dry_run == 1 ]]; then
  args+=(scan)
else
  args+=(import)
fi
args+=(--source zsh --histfile "$histfile")
[[ $json == 1 ]] && args+=(--json --compact)
[[ -n $limit ]] && args+=(--limit "$limit")
[[ -n $trufflehog ]] && args+=(--trufflehog "$trufflehog")
# `scan` has no --namespace, and only import needs the no-picker flag.
if [[ $dry_run == 0 ]]; then
  args+=(--all)
  [[ -n $namespace ]] && args+=(--namespace "$namespace")
fi

if [[ $json == 0 ]]; then
  printf 'using %s\n' "$hoard"
  if [[ $dry_run == 1 ]]; then
    printf 'scanning %s (dry run, nothing will be written)\n' "$histfile"
  else
    printf 'importing new commands from %s\n' "$histfile"
  fi
fi

exec "$hoard" "${args[@]}"
