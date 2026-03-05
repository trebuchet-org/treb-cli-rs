#!/bin/bash
# Ralph Wiggum - Long-running AI agent loop
# Usage: ./ralph.sh [status] [--tool amp|claude] [--model MODEL] [max_iterations]

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

# ---------------------------------------------------------------------------
# Path setup (before argument parsing so status can use it)
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RALPH_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

STATE_DIR="$SCRIPT_DIR/current"       # symlink to active state dir
PRD_FILE="$STATE_DIR/prd.json"
PROGRESS_FILE="$STATE_DIR/progress.txt"
LAST_BRANCH_FILE="$STATE_DIR/.last-branch"
ARCHIVE_DIR="$SCRIPT_DIR/archive"

# ---------------------------------------------------------------------------
# Status subcommand
# ---------------------------------------------------------------------------
if [[ "${1:-}" == "status" ]]; then
  echo ""
  echo "Ralph Status"
  echo "════════════════════════════════════════════"
  echo ""

  # Check if current symlink exists
  if [[ ! -L "$SCRIPT_DIR/current" ]]; then
    echo "  No active plan. Run mega-ralph.sh or use /ralph skill to set one up."
    echo ""
    echo "════════════════════════════════════════════"
    exit 0
  fi

  # Determine plan ID from symlink target
  CURRENT_TARGET=$(readlink "$SCRIPT_DIR/current" 2>/dev/null || echo "")
  PLAN_ID=$(basename "$CURRENT_TARGET")

  # Read masterplan.json if exists (for mega-ralph plans)
  MASTERPLAN_FILE="$STATE_DIR/masterplan.json"
  if [[ -f "$MASTERPLAN_FILE" ]]; then
    PLAN_NAME=$(jq -r '.project // "Unknown"' "$MASTERPLAN_FILE" 2>/dev/null || echo "Unknown")
    TOTAL_PHASES=$(jq -r '.totalPhases // "?"' "$MASTERPLAN_FILE" 2>/dev/null || echo "?")
    CURRENT_PHASE=$(jq -r '.currentPhase // "?"' "$MASTERPLAN_FILE" 2>/dev/null || echo "?")
    PHASE_TITLE=$(jq -r --argjson p "${CURRENT_PHASE}" \
      '(.phases[] | select(.phase == $p) | .title) // "Unknown"' \
      "$MASTERPLAN_FILE" 2>/dev/null || echo "Unknown")
    echo "  Active Plan:  $PLAN_ID - $PLAN_NAME"
    echo "  Phase:        $CURRENT_PHASE of $TOTAL_PHASES ($PHASE_TITLE)"
  else
    echo "  Active Plan:  $PLAN_ID (standalone)"
  fi

  # Read branch from prd.json
  if [[ -f "$PRD_FILE" ]]; then
    BRANCH=$(jq -r '.branchName // "unknown"' "$PRD_FILE" 2>/dev/null || echo "unknown")
    echo "  Branch:       $BRANCH"
  fi

  echo ""

  # Stories status
  if [[ -f "$PRD_FILE" ]]; then
    TOTAL_STORIES=$(jq '.userStories | length' "$PRD_FILE" 2>/dev/null || echo "0")
    DONE_STORIES=$(jq '[.userStories[] | select(.passes == true)] | length' "$PRD_FILE" 2>/dev/null || echo "0")
    FIRST_PENDING=$(jq -r '[.userStories[] | select(.passes == false)][0].id // empty' "$PRD_FILE" 2>/dev/null || echo "")

    echo "  Stories:      $DONE_STORIES/$TOTAL_STORIES complete"

    # List each story with status marker: ✓ done, → next, · pending
    jq -r --arg pending "$FIRST_PENDING" '.userStories[] |
      if .passes == true then "    ✓ \(.id)  \(.title)"
      elif .id == $pending then "    → \(.id)  \(.title)"
      else "    · \(.id)  \(.title)"
      end' "$PRD_FILE" 2>/dev/null
  else
    echo "  Stories:      No prd.json found"
  fi

  echo ""

  # Last commit
  LAST_COMMIT=$(git log --oneline -1 2>/dev/null || echo "(no commits)")
  LAST_TIME=$(git log -1 --format='%ar' 2>/dev/null || echo "")
  echo "  Last Commit:  $LAST_COMMIT${LAST_TIME:+ ($LAST_TIME)}"

  # Progress file location
  echo "  Progress:     $STATE_DIR/progress.txt"

  echo ""
  echo "════════════════════════════════════════════"
  exit 0
fi

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
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
# Ensure state directory exists (via current symlink)
# ---------------------------------------------------------------------------
# If current symlink doesn't exist, create a default state dir
if [[ ! -L "$SCRIPT_DIR/current" ]]; then
  mkdir -p "$SCRIPT_DIR/state/default"
  ln -sfn "state/default" "$SCRIPT_DIR/current"
fi

# Ensure the target of current exists
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
echo "Interjection: echo 'your notes' > $STATE_DIR/interjection.md"

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

  # Check for interjection file — user can write to this between iterations
  INTERJECTION_FILE="$STATE_DIR/interjection.md"
  INTERJECTION=""
  if [[ -f "$INTERJECTION_FILE" ]] && [[ -s "$INTERJECTION_FILE" ]]; then
    INTERJECTION=$(cat "$INTERJECTION_FILE")
    echo ""
    echo "  ** Interjection detected — incorporating user notes **"
    echo ""
    # Clear the file after reading
    > "$INTERJECTION_FILE"
  fi

  # Build the prompt, prepending interjection if present
  PROMPT_FILE=$(mktemp)
  if [[ "$TOOL" == "amp" ]]; then
    if [[ -n "$INTERJECTION" ]]; then
      printf '## IMPORTANT — User Interjection\n\nThe user has added the following notes before this iteration. Take these into account and prioritize them:\n\n%s\n\n---\n\n' "$INTERJECTION" > "$PROMPT_FILE"
      cat "$SCRIPT_DIR/prompt.md" >> "$PROMPT_FILE"
    else
      cp "$SCRIPT_DIR/prompt.md" "$PROMPT_FILE"
    fi
  else
    if [[ -n "$INTERJECTION" ]]; then
      printf '## IMPORTANT — User Interjection\n\nThe user has added the following notes before this iteration. Take these into account and prioritize them:\n\n%s\n\n---\n\n' "$INTERJECTION" > "$PROMPT_FILE"
      cat "$SCRIPT_DIR/CLAUDE.md" >> "$PROMPT_FILE"
    else
      cp "$SCRIPT_DIR/CLAUDE.md" "$PROMPT_FILE"
    fi
  fi

  # Run the tool — output goes to terminal AND temp file (no subshell)
  EXIT_CODE=0
  if [[ "$TOOL" == "amp" ]]; then
    cat "$PROMPT_FILE" | amp --dangerously-allow-all 2>&1 | tee "$OUTFILE" || EXIT_CODE=$?
  else
    claude --dangerously-skip-permissions $CLAUDE_MODEL_ARGS --print < "$PROMPT_FILE" 2>&1 | tee "$OUTFILE" || EXIT_CODE=$?
  fi
  rm -f "$PROMPT_FILE"

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
