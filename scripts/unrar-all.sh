#!/usr/bin/env bash
set -euo pipefail

# unrar-all.sh — Extract all .rar files in a directory (or a specific file)
#
# Uses unar (primary) or 7z (fallback) to handle RAR archives including RAR5.
#
# Usage:
#   ./unrar-all.sh [options] [directory | --file RARFILE]
#
# Options:
#   -f, --file RARFILE     Extract a single .rar file
#   -p, --password PASS    Password for encrypted RAR archives
#   -r, --recursive        Search recursively for .rar files
#   -o, --output-dir DIR   Output directory (default: same directory as each .rar)
#   -h, --help             Show this help and exit
#
# If directory is omitted (and no --file), defaults to current directory.

usage() {
  sed -n '/^# Usage:/,/^$/{ s/^#[[:space:]]\{0,1\}//; p; }' "$0"
  exit "${1:-0}"
}

PASSWORD=""
RECURSIVE=false
OUTPUT_DIR=""
DIR=""
SINGLE_FILE=""

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    -f|--file)
      shift
      SINGLE_FILE="$1"
      ;;
    -p|--password)
      shift
      PASSWORD="$1"
      ;;
    -r|--recursive)
      RECURSIVE=true
      ;;
    -o|--output-dir)
      shift
      OUTPUT_DIR="$1"
      ;;
    -h|--help)
      usage 0
      ;;
    -*)
      echo "Error: unknown option $1" >&2
      usage 1
      ;;
    *)
      if [[ -z "$DIR" ]]; then
        DIR="$1"
      else
        echo "Error: unexpected argument $1" >&2
        usage 1
      fi
      ;;
  esac
  shift
done

# Handle single file mode
if [[ -n "$SINGLE_FILE" ]]; then
  if [[ ! -f "$SINGLE_FILE" ]]; then
    echo "Error: file not found: $SINGLE_FILE" >&2
    exit 1
  fi
  RAR_FILES=("$SINGLE_FILE")
else
  # Default to current directory
  DIR="${DIR:-.}"

  if [[ ! -d "$DIR" ]]; then
    echo "Error: directory not found: $DIR" >&2
    exit 1
  fi

  # Gather RAR files
  RAR_FILES=()
  if [[ "$RECURSIVE" == true ]]; then
    while IFS= read -r -d '' f; do
      RAR_FILES+=("$f")
    done < <(find "$DIR" -type f -iname '*.rar' -print0 | sort -z)
  else
    while IFS= read -r -d '' f; do
      RAR_FILES+=("$f")
    done < <(find "$DIR" -maxdepth 1 -type f -iname '*.rar' -print0 | sort -z)
  fi

  if [[ ${#RAR_FILES[@]} -eq 0 ]]; then
    echo "No .rar files found in $DIR" >&2
    exit 0
  fi
fi

if [[ -n "$SINGLE_FILE" ]]; then
  echo "Extracting: $SINGLE_FILE"
else
  echo "Found ${#RAR_FILES[@]} .rar file(s) in $DIR"
fi
echo ""

EXTRACT_OK=0
EXTRACT_FAIL=0

for rar in "${RAR_FILES[@]}"; do
  BASENAME="$(basename "$rar")"
  RAR_DIR="$(dirname "$rar")"

  # Determine output directory for this archive
  if [[ -n "$OUTPUT_DIR" ]]; then
    TARGET_DIR="$OUTPUT_DIR"
  else
    TARGET_DIR="$RAR_DIR"
  fi
  mkdir -p "$TARGET_DIR"

  echo "━━━ Extracting: $BASENAME"

  EXTRACT_OUTPUT="$(mktemp)"

  # Try unar first (handles RAR5, FLAC, etc.), fall back to 7z
  UNAR_CMD=(unar -q -f -o "$TARGET_DIR")
  if [[ -n "$PASSWORD" ]]; then
    UNAR_CMD+=(-p "$PASSWORD")
  fi
  UNAR_CMD+=("$rar")

  if "${UNAR_CMD[@]}" > "$EXTRACT_OUTPUT" 2>&1; then
    echo "  ✓ Done → $TARGET_DIR"
    rm -f "$EXTRACT_OUTPUT"
    EXTRACT_OK=$((EXTRACT_OK + 1))
    continue
  fi

  # unar failed — try 7z as fallback
  SEVEN_CMD=(7z x -y)
  if [[ -n "$PASSWORD" ]]; then
    SEVEN_CMD+=("-p$PASSWORD")
  fi
  SEVEN_CMD+=("-o$TARGET_DIR" "$rar")

  if "${SEVEN_CMD[@]}" > "$EXTRACT_OUTPUT" 2>&1; then
    echo "  ✓ Done (via 7z) → $TARGET_DIR"
    rm -f "$EXTRACT_OUTPUT"
    EXTRACT_OK=$((EXTRACT_OK + 1))
  else
    # Show error summary
    grep -i 'error\|wrong\|unsupported\|cannot\|fail\|wrong password' "$EXTRACT_OUTPUT" 2>/dev/null \
      | head -10 | sed 's/^/  /'
    rm -f "$EXTRACT_OUTPUT"
    echo "  ✗ Failed"
    EXTRACT_FAIL=$((EXTRACT_FAIL + 1))
  fi
done

echo ""
echo "━━━ Summary: ${EXTRACT_OK} succeeded, ${EXTRACT_FAIL} failed"

if [[ "$EXTRACT_FAIL" -gt 0 ]]; then
  exit 1
fi
