#!/bin/bash
# Ralph Wiggum - Long-running AI agent loop
# Usage: ./ralph.sh [--tool amp|claude] [--model MODEL] [max_iterations]

# ---------------------------------------------------------------------------
# Signal handling - ensure Ctrl-C kills everything
# ---------------------------------------------------------------------------
OUTFILE=""
cleanup() {
  echo ""
  echo "Interrupted."
  rm -f "$OUTFILE"
  # Remove trap to avoid recursion, then kill process group
  trap - INT TERM
  kill 0 2>/dev/null
  exit 130
}
trap cleanup INT TERM

set -e

# Parse arguments
TOOL="amp"  # Default to amp for backwards compatibility
MAX_ITERATIONS=10
MODEL=""

while [[ $# -gt 0 ]]; do
  case $1 in
    --tool)
      TOOL="$2"
      shift 2
      ;;
    --tool=*)
      TOOL="${1#*=}"
      shift
      ;;
    --model)
      MODEL="$2"
      shift 2
      ;;
    --model=*)
      MODEL="${1#*=}"
      shift
      ;;
    *)
      # Assume it's max_iterations if it's a number
      if [[ "$1" =~ ^[0-9]+$ ]]; then
        MAX_ITERATIONS="$1"
      fi
      shift
      ;;
  esac
done

# Build model args for claude CLI
CLAUDE_MODEL_ARGS=""
if [[ -n "$MODEL" ]]; then
  CLAUDE_MODEL_ARGS="--model $MODEL"
fi

# Validate tool choice
if [[ "$TOOL" != "amp" && "$TOOL" != "claude" ]]; then
  echo "Error: Invalid tool '$TOOL'. Must be 'amp' or 'claude'."
  exit 1
fi

# ---------------------------------------------------------------------------
# Path setup
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RALPH_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

STATE_DIR="$RALPH_ROOT/.state"
PRD_FILE="$STATE_DIR/prd.json"
PROGRESS_FILE="$STATE_DIR/progress.txt"
LAST_BRANCH_FILE="$STATE_DIR/.last-branch"
ARCHIVE_DIR="$RALPH_ROOT/archive"

# Ensure .state/ directory exists
mkdir -p "$STATE_DIR"

# Archive previous run if branch changed
if [ -f "$PRD_FILE" ] && [ -f "$LAST_BRANCH_FILE" ]; then
  CURRENT_BRANCH=$(jq -r '.branchName // empty' "$PRD_FILE" 2>/dev/null || echo "")
  LAST_BRANCH=$(cat "$LAST_BRANCH_FILE" 2>/dev/null || echo "")

  if [ -n "$CURRENT_BRANCH" ] && [ -n "$LAST_BRANCH" ] && [ "$CURRENT_BRANCH" != "$LAST_BRANCH" ]; then
    # Archive the previous run
    DATE=$(date +%Y-%m-%d)
    # Strip "ralph/" prefix from branch name for folder
    FOLDER_NAME=$(echo "$LAST_BRANCH" | sed 's|^ralph/||')
    ARCHIVE_FOLDER="$ARCHIVE_DIR/$DATE-$FOLDER_NAME"

    echo "Archiving previous run: $LAST_BRANCH"
    mkdir -p "$ARCHIVE_FOLDER"
    [ -f "$PRD_FILE" ] && cp "$PRD_FILE" "$ARCHIVE_FOLDER/"
    [ -f "$PROGRESS_FILE" ] && cp "$PROGRESS_FILE" "$ARCHIVE_FOLDER/"
    echo "   Archived to: $ARCHIVE_FOLDER"

    # Reset progress file for new run
    echo "# Ralph Progress Log" > "$PROGRESS_FILE"
    echo "Started: $(date)" >> "$PROGRESS_FILE"
    echo "---" >> "$PROGRESS_FILE"
  fi
fi

# Track current branch
if [ -f "$PRD_FILE" ]; then
  CURRENT_BRANCH=$(jq -r '.branchName // empty' "$PRD_FILE" 2>/dev/null || echo "")
  if [ -n "$CURRENT_BRANCH" ]; then
    echo "$CURRENT_BRANCH" > "$LAST_BRANCH_FILE"
  fi
fi

# Initialize progress file if it doesn't exist
if [ ! -f "$PROGRESS_FILE" ]; then
  echo "# Ralph Progress Log" > "$PROGRESS_FILE"
  echo "Started: $(date)" >> "$PROGRESS_FILE"
  echo "---" >> "$PROGRESS_FILE"
fi

if [[ -n "$MODEL" ]]; then
  echo "Starting Ralph - Tool: $TOOL - Model: $MODEL - Max iterations: $MAX_ITERATIONS"
else
  echo "Starting Ralph - Tool: $TOOL - Max iterations: $MAX_ITERATIONS"
fi

# ---------------------------------------------------------------------------
# Exponential backoff settings
# ---------------------------------------------------------------------------
BACKOFF=5        # Start at 5 seconds
MAX_BACKOFF=300  # Cap at 5 minutes

# Temp file for capturing output (avoids subshell signal issues)
OUTFILE=$(mktemp)

for i in $(seq 1 $MAX_ITERATIONS); do
  echo ""
  echo "==============================================================="
  echo "  Ralph Iteration $i of $MAX_ITERATIONS ($TOOL)"
  echo "==============================================================="

  # Run the tool — output goes to terminal AND temp file (no subshell)
  EXIT_CODE=0
  if [[ "$TOOL" == "amp" ]]; then
    cat "$SCRIPT_DIR/prompt.md" | amp --dangerously-allow-all 2>&1 | tee "$OUTFILE" || EXIT_CODE=$?
  else
    claude --dangerously-skip-permissions $CLAUDE_MODEL_ARGS --print < "$SCRIPT_DIR/CLAUDE.md" 2>&1 | tee "$OUTFILE" || EXIT_CODE=$?
  fi

  # Exit immediately on SIGINT/SIGTERM (exit code 130 or 143)
  if [[ $EXIT_CODE -eq 130 || $EXIT_CODE -eq 143 ]]; then
    echo ""
    echo "Interrupted."
    rm -f "$OUTFILE"
    exit $EXIT_CODE
  fi

  # Check for completion signal
  if grep -q "<promise>COMPLETE</promise>" "$OUTFILE" 2>/dev/null; then
    echo ""
    echo "Ralph completed all tasks!"
    echo "Completed at iteration $i of $MAX_ITERATIONS"
    rm -f "$OUTFILE"
    exit 0
  fi

  # Check for errors and apply exponential backoff
  if [[ $EXIT_CODE -ne 0 ]]; then
    echo ""
    echo "Error on iteration $i (exit code $EXIT_CODE). Retrying in ${BACKOFF}s..."
    sleep "$BACKOFF"
    BACKOFF=$((BACKOFF * 2))
    if [[ $BACKOFF -gt $MAX_BACKOFF ]]; then
      BACKOFF=$MAX_BACKOFF
    fi
    continue
  fi

  # Reset backoff on success
  BACKOFF=5

  echo "Iteration $i complete. Continuing..."
  sleep 2
done

rm -f "$OUTFILE"
echo ""
echo "Ralph reached max iterations ($MAX_ITERATIONS) without completing all tasks."
echo "Check $PROGRESS_FILE for status."
exit 1
