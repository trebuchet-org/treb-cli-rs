#!/bin/bash
# mega-ralph.sh - Multi-Phase Project Orchestrator for Ralph
#
# Orchestrates a large multi-phase project by reading a MASTER_PLAN.md,
# generating a PRD for each phase, converting it to prd.json, and running
# ralph.sh to execute each phase autonomously.
#
# Usage:
#   ./mega-ralph.sh [--plan MASTER_PLAN.md] [--start-phase N] [--max-iterations-per-phase N] [--tool amp|claude]

set -e

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
PLAN_FILE="MASTER_PLAN.md"
START_PHASE=1
MAX_ITERATIONS=25
TOOL="claude"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case $1 in
    --plan)
      PLAN_FILE="$2"
      shift 2
      ;;
    --plan=*)
      PLAN_FILE="${1#*=}"
      shift
      ;;
    --start-phase)
      START_PHASE="$2"
      shift 2
      ;;
    --start-phase=*)
      START_PHASE="${1#*=}"
      shift
      ;;
    --max-iterations-per-phase)
      MAX_ITERATIONS="$2"
      shift 2
      ;;
    --max-iterations-per-phase=*)
      MAX_ITERATIONS="${1#*=}"
      shift
      ;;
    --tool)
      TOOL="$2"
      shift 2
      ;;
    --tool=*)
      TOOL="${1#*=}"
      shift
      ;;
    -h|--help)
      echo "Usage: mega-ralph.sh [OPTIONS]"
      echo ""
      echo "Options:"
      echo "  --plan FILE                   Master plan file (default: MASTER_PLAN.md)"
      echo "  --start-phase N               Resume from phase N (default: 1)"
      echo "  --max-iterations-per-phase N  Max ralph iterations per phase (default: 25)"
      echo "  --tool amp|claude             AI tool to use (default: claude)"
      echo "  -h, --help                    Show this help"
      exit 0
      ;;
    *)
      echo "Error: Unknown argument '$1'. Use --help for usage."
      exit 1
      ;;
  esac
done

# ---------------------------------------------------------------------------
# Validate
# ---------------------------------------------------------------------------
if [[ "$TOOL" != "amp" && "$TOOL" != "claude" ]]; then
  echo "Error: Invalid tool '$TOOL'. Must be 'amp' or 'claude'."
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLAN_PATH="$SCRIPT_DIR/$PLAN_FILE"
MEGA_PROGRESS="$SCRIPT_DIR/mega-progress.json"
TASKS_DIR="$SCRIPT_DIR/tasks"
PRD_PROMPT_TEMPLATE="$SCRIPT_DIR/mega-claude-prompt.md"
CONVERT_PROMPT_TEMPLATE="$SCRIPT_DIR/mega-ralph-convert-prompt.md"

if [[ ! -f "$PLAN_PATH" ]]; then
  echo "Error: Master plan not found at $PLAN_PATH"
  echo "Create a MASTER_PLAN.md or specify one with --plan FILE"
  exit 1
fi

if [[ ! -f "$PRD_PROMPT_TEMPLATE" ]]; then
  echo "Error: PRD prompt template not found at $PRD_PROMPT_TEMPLATE"
  exit 1
fi

if [[ ! -f "$CONVERT_PROMPT_TEMPLATE" ]]; then
  echo "Error: Conversion prompt template not found at $CONVERT_PROMPT_TEMPLATE"
  exit 1
fi

if ! command -v jq &>/dev/null; then
  echo "Error: jq is required but not installed."
  exit 1
fi

if ! command -v claude &>/dev/null; then
  echo "Error: claude CLI is required but not installed."
  exit 1
fi

if ! command -v python3 &>/dev/null; then
  echo "Error: python3 is required but not installed (used for template expansion)."
  exit 1
fi

# ---------------------------------------------------------------------------
# Parse the master plan to extract phases
#
# Expects phases formatted as:
#   ## Phase 1: Title
#   Description text ...
#
#   ## Phase 2: Title
#   Description text ...
#
# This parser extracts phase numbers, titles, and description blocks.
# ---------------------------------------------------------------------------
parse_phases() {
  local plan_file="$1"
  local phases_json="[]"
  local current_phase=""
  local current_title=""
  local current_desc=""
  local in_phase=false

  while IFS= read -r line || [[ -n "$line" ]]; do
    # Match "## Phase N: Title" or "## Phase N - Title" or "## Phase N -- Title"
    if [[ "$line" =~ ^##[[:space:]]+[Pp]hase[[:space:]]+([0-9]+)[[:space:]]*[:.]+[[:space:]]*(.*)|^##[[:space:]]+[Pp]hase[[:space:]]+([0-9]+)[[:space:]]*[-]+[[:space:]]+(.*) ]]; then
      # Handle both regex groups (: or - delimiter)
      if [[ -n "${BASH_REMATCH[1]}" ]]; then
        _phase="${BASH_REMATCH[1]}"
        _title="${BASH_REMATCH[2]}"
      else
        _phase="${BASH_REMATCH[3]}"
        _title="${BASH_REMATCH[4]}"
      fi
      # Save previous phase if we have one
      if $in_phase && [[ -n "$current_phase" ]]; then
        # Trim trailing whitespace from description
        current_desc=$(echo "$current_desc" | sed 's/[[:space:]]*$//')
        phases_json=$(echo "$phases_json" | jq \
          --arg num "$current_phase" \
          --arg title "$current_title" \
          --arg desc "$current_desc" \
          '. + [{"phase": ($num | tonumber), "title": $title, "description": $desc}]')
      fi
      current_phase="$_phase"
      current_title="$_title"
      current_desc=""
      in_phase=true
    elif $in_phase; then
      # Accumulate description lines
      if [[ -n "$current_desc" ]]; then
        current_desc="$current_desc
$line"
      else
        # Skip leading empty lines in description
        if [[ -n "$line" ]]; then
          current_desc="$line"
        fi
      fi
    fi
  done < "$plan_file"

  # Save the last phase
  if $in_phase && [[ -n "$current_phase" ]]; then
    current_desc=$(echo "$current_desc" | sed 's/[[:space:]]*$//')
    phases_json=$(echo "$phases_json" | jq \
      --arg num "$current_phase" \
      --arg title "$current_title" \
      --arg desc "$current_desc" \
      '. + [{"phase": ($num | tonumber), "title": $title, "description": $desc}]')
  fi

  echo "$phases_json"
}

# ---------------------------------------------------------------------------
# Initialize or load mega-progress.json
# ---------------------------------------------------------------------------
init_progress() {
  local total_phases="$1"
  local project_name

  # Derive project name from plan filename or directory
  project_name=$(basename "$(pwd)" | sed 's/[^a-zA-Z0-9_-]/-/g')

  if [[ -f "$MEGA_PROGRESS" ]]; then
    echo "Resuming from existing mega-progress.json"
    return
  fi

  cat > "$MEGA_PROGRESS" <<EOJSON
{
  "project": "$project_name",
  "masterPlan": "$PLAN_FILE",
  "totalPhases": $total_phases,
  "currentPhase": $START_PHASE,
  "phases": []
}
EOJSON

  echo "Created mega-progress.json for $total_phases phases"
}

# ---------------------------------------------------------------------------
# Update mega-progress.json
# ---------------------------------------------------------------------------
update_progress_start() {
  local phase_num="$1"
  local phase_title="$2"
  local branch_name="$3"
  local started_at
  started_at=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  # Check if phase entry already exists
  local existing
  existing=$(jq --argjson p "$phase_num" '.phases[] | select(.phase == $p)' "$MEGA_PROGRESS" 2>/dev/null || echo "")

  if [[ -n "$existing" ]]; then
    # Update existing entry
    jq --argjson p "$phase_num" \
       --arg status "in_progress" \
       --arg started "$started_at" \
       --arg branch "$branch_name" \
       '(.phases[] | select(.phase == $p)) |= . + {
         "status": $status,
         "startedAt": $started,
         "branch": $branch
       } | .currentPhase = $p' "$MEGA_PROGRESS" > "$MEGA_PROGRESS.tmp" && mv "$MEGA_PROGRESS.tmp" "$MEGA_PROGRESS"
  else
    # Add new entry
    jq --argjson p "$phase_num" \
       --arg title "$phase_title" \
       --arg status "in_progress" \
       --arg started "$started_at" \
       --arg branch "$branch_name" \
       '.phases += [{
         "phase": $p,
         "title": $title,
         "status": $status,
         "startedAt": $started,
         "completedAt": null,
         "iterations": 0,
         "branch": $branch
       }] | .currentPhase = $p' "$MEGA_PROGRESS" > "$MEGA_PROGRESS.tmp" && mv "$MEGA_PROGRESS.tmp" "$MEGA_PROGRESS"
  fi
}

update_progress_complete() {
  local phase_num="$1"
  local iterations="$2"
  local completed_at
  completed_at=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  jq --argjson p "$phase_num" \
     --arg status "completed" \
     --arg completed "$completed_at" \
     --argjson iters "$iterations" \
     '(.phases[] | select(.phase == $p)) |= . + {
       "status": $status,
       "completedAt": $completed,
       "iterations": $iters
     }' "$MEGA_PROGRESS" > "$MEGA_PROGRESS.tmp" && mv "$MEGA_PROGRESS.tmp" "$MEGA_PROGRESS"
}

update_progress_failed() {
  local phase_num="$1"
  local iterations="$2"

  jq --argjson p "$phase_num" \
     --arg status "failed" \
     --argjson iters "$iterations" \
     '(.phases[] | select(.phase == $p)) |= . + {
       "status": $status,
       "iterations": $iters
     }' "$MEGA_PROGRESS" > "$MEGA_PROGRESS.tmp" && mv "$MEGA_PROGRESS.tmp" "$MEGA_PROGRESS"
}

# ---------------------------------------------------------------------------
# Get previous phases summary from git log and progress
# ---------------------------------------------------------------------------
get_previous_phases_summary() {
  local current_phase="$1"
  local summary=""

  if [[ "$current_phase" -le 1 ]]; then
    echo "This is the first phase. No previous phases."
    return
  fi

  # Gather completed phase info from mega-progress.json
  local completed_phases
  completed_phases=$(jq -r --argjson p "$current_phase" \
    '.phases[] | select(.phase < $p and .status == "completed") | "Phase \(.phase): \(.title) [branch: \(.branch)]"' \
    "$MEGA_PROGRESS" 2>/dev/null || echo "")

  if [[ -n "$completed_phases" ]]; then
    summary="Completed phases:
$completed_phases
"
  fi

  # Get recent git log (last 30 commits) for context on what was built
  local git_log
  git_log=$(git log --oneline -30 2>/dev/null || echo "(no git history available)")

  if [[ -n "$git_log" ]]; then
    summary="${summary}
Recent git history:
$git_log"
  fi

  echo "$summary"
}

# ---------------------------------------------------------------------------
# Generate a branch name from phase number and title
# ---------------------------------------------------------------------------
make_branch_name() {
  local phase_num="$1"
  local phase_title="$2"

  # Convert title to kebab-case: lowercase, replace spaces/special chars with hyphens
  local slug
  slug=$(echo "$phase_title" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/-/g' | sed 's/--*/-/g' | sed 's/^-//' | sed 's/-$//')

  # Pad phase number to 2 digits
  printf "ralph/phase-%02d-%s" "$phase_num" "$slug"
}

# ---------------------------------------------------------------------------
# Build a prompt by replacing placeholders in a template using temp files
# for robust multi-line content handling.
# ---------------------------------------------------------------------------
build_prompt() {
  local template_file="$1"
  local phase_number="$2"
  local phase_title="$3"
  local phase_description="$4"
  local previous_summary="$5"
  local project_name="$6"
  local prd_file="${7:-}"

  local output_file
  output_file=$(mktemp)

  # Write placeholder values to temp files for safe insertion
  local plan_file_tmp desc_file summary_file prd_file_tmp
  plan_file_tmp=$(mktemp)
  desc_file=$(mktemp)
  summary_file=$(mktemp)
  prd_file_tmp=$(mktemp)

  cat "$PLAN_PATH" > "$plan_file_tmp"
  printf '%s' "$phase_description" > "$desc_file"
  printf '%s' "$previous_summary" > "$summary_file"
  printf '%s' "$prd_file" > "$prd_file_tmp"

  # Use Python for safe template substitution (handles all special chars)
  python3 -c "
import sys
template = open('$template_file').read()
replacements = {
    '{{PHASE_NUMBER}}': '$phase_number',
    '{{PHASE_TITLE}}': '$phase_title',
    '{{PROJECT_NAME}}': '$project_name',
    '{{MASTER_PLAN}}': open('$plan_file_tmp').read(),
    '{{PHASE_DESCRIPTION}}': open('$desc_file').read(),
    '{{PREVIOUS_PHASES_SUMMARY}}': open('$summary_file').read(),
    '{{PRD_FILE}}': open('$prd_file_tmp').read().strip(),
}
for key, val in replacements.items():
    template = template.replace(key, val)
sys.stdout.write(template)
" > "$output_file"

  cat "$output_file"
  rm -f "$output_file" "$plan_file_tmp" "$desc_file" "$summary_file" "$prd_file_tmp"
}

# ---------------------------------------------------------------------------
# Archive a completed phase
# ---------------------------------------------------------------------------
archive_phase() {
  local phase_num="$1"
  local phase_title="$2"
  local branch_name="$3"
  local archive_dir="$SCRIPT_DIR/archive"

  local date_str
  date_str=$(date +%Y-%m-%d)
  local folder_name
  folder_name=$(echo "$branch_name" | sed 's|^ralph/||')
  local archive_path="$archive_dir/$date_str-$folder_name"

  echo "Archiving phase $phase_num: $phase_title"
  mkdir -p "$archive_path"

  # Archive prd.json and progress.txt
  [[ -f "$SCRIPT_DIR/prd.json" ]] && cp "$SCRIPT_DIR/prd.json" "$archive_path/"
  [[ -f "$SCRIPT_DIR/progress.txt" ]] && cp "$SCRIPT_DIR/progress.txt" "$archive_path/"

  # Archive the phase PRD markdown if it exists
  local prd_pattern="$TASKS_DIR/prd-phase-$(printf '%02d' "$phase_num")-*.md"
  for f in $prd_pattern; do
    [[ -f "$f" ]] && cp "$f" "$archive_path/"
  done

  echo "  Archived to: $archive_path"

  # Reset progress.txt for the next phase
  echo "# Ralph Progress Log" > "$SCRIPT_DIR/progress.txt"
  echo "Started: $(date)" >> "$SCRIPT_DIR/progress.txt"
  echo "---" >> "$SCRIPT_DIR/progress.txt"
}

# ---------------------------------------------------------------------------
# Generate a PRD for a single phase using Claude
# ---------------------------------------------------------------------------
generate_phase_prd() {
  local phase_num="$1"
  local phase_title="$2"
  local phase_description="$3"
  local previous_summary="$4"
  local project_name="$5"

  local padded_phase
  padded_phase=$(printf '%02d' "$phase_num")
  local title_slug
  title_slug=$(echo "$phase_title" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/-/g' | sed 's/--*/-/g' | sed 's/^-//' | sed 's/-$//')
  local prd_filename="prd-phase-${padded_phase}-${title_slug}.md"
  local prd_path="$TASKS_DIR/$prd_filename"

  # If PRD already exists, skip generation
  if [[ -f "$prd_path" ]]; then
    echo "  PRD already exists: $prd_path (skipping generation)"
    echo "$prd_path"
    return
  fi

  echo "  Generating PRD for Phase $phase_num: $phase_title ..."

  mkdir -p "$TASKS_DIR"

  # Build the prompt from the template
  local prompt
  prompt=$(build_prompt "$PRD_PROMPT_TEMPLATE" "$phase_num" "$phase_title" "$phase_description" "$previous_summary" "$project_name")

  # Invoke Claude to generate the PRD
  local output
  output=$(claude --dangerously-skip-permissions --print -p "$prompt" 2>&1) || {
    echo "Error: Claude failed to generate PRD for phase $phase_num"
    echo "$output"
    return 1
  }

  # Verify the PRD file was created by Claude
  if [[ ! -f "$prd_path" ]]; then
    # Claude may have output the PRD to stdout instead of saving it.
    # Save it ourselves as a fallback.
    echo "$output" > "$prd_path"
    echo "  PRD saved (from stdout fallback): $prd_path"
  else
    echo "  PRD generated: $prd_path"
  fi

  echo "$prd_path"
}

# ---------------------------------------------------------------------------
# Convert a phase PRD to prd.json using Claude
# ---------------------------------------------------------------------------
convert_prd_to_json() {
  local prd_path="$1"
  local phase_num="$2"
  local phase_title="$3"
  local project_name="$4"

  echo "  Converting PRD to prd.json ..."

  local padded_phase
  padded_phase=$(printf '%02d' "$phase_num")
  local title_slug
  title_slug=$(echo "$phase_title" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/-/g' | sed 's/--*/-/g' | sed 's/^-//' | sed 's/-$//')

  # Build the conversion prompt from the template
  local prompt
  prompt=$(build_prompt "$CONVERT_PROMPT_TEMPLATE" "$phase_num" "$phase_title" "" "" "$project_name" "$prd_path")

  # Invoke Claude to convert the PRD
  local output
  output=$(claude --dangerously-skip-permissions --print -p "$prompt" 2>&1) || {
    echo "Error: Claude failed to convert PRD to prd.json"
    echo "$output"
    return 1
  }

  # Verify prd.json was created
  if [[ ! -f "$SCRIPT_DIR/prd.json" ]]; then
    echo "Error: prd.json was not created after conversion"
    return 1
  fi

  # Validate it is proper JSON
  if ! jq empty "$SCRIPT_DIR/prd.json" 2>/dev/null; then
    echo "Error: prd.json is not valid JSON"
    return 1
  fi

  echo "  prd.json created successfully"
}

# ---------------------------------------------------------------------------
# Main execution
# ---------------------------------------------------------------------------
echo ""
echo "================================================================"
echo "  MEGA-RALPH - Multi-Phase Project Orchestrator"
echo "================================================================"
echo "  Plan:       $PLAN_FILE"
echo "  Tool:       $TOOL"
echo "  Start:      Phase $START_PHASE"
echo "  Max Iters:  $MAX_ITERATIONS per phase"
echo "================================================================"
echo ""

# Parse the master plan
echo "Parsing master plan: $PLAN_PATH"
PHASES_JSON=$(parse_phases "$PLAN_PATH")
TOTAL_PHASES=$(echo "$PHASES_JSON" | jq 'length')

if [[ "$TOTAL_PHASES" -eq 0 ]]; then
  echo "Error: No phases found in $PLAN_PATH"
  echo "Ensure phases are formatted as: ## Phase N: Title"
  exit 1
fi

echo "Found $TOTAL_PHASES phases"
echo ""

# Derive project name
PROJECT_NAME=$(basename "$(pwd)" | sed 's/[^a-zA-Z0-9_-]/-/g')

# Initialize progress tracking
init_progress "$TOTAL_PHASES"

# ---------------------------------------------------------------------------
# Phase loop
# ---------------------------------------------------------------------------
for (( phase_idx=0; phase_idx < TOTAL_PHASES; phase_idx++ )); do
  PHASE_NUM=$(echo "$PHASES_JSON" | jq -r ".[$phase_idx].phase")
  PHASE_TITLE=$(echo "$PHASES_JSON" | jq -r ".[$phase_idx].title")
  PHASE_DESC=$(echo "$PHASES_JSON" | jq -r ".[$phase_idx].description")

  # Skip phases before the start phase
  if [[ "$PHASE_NUM" -lt "$START_PHASE" ]]; then
    echo "Skipping Phase $PHASE_NUM: $PHASE_TITLE (before start phase $START_PHASE)"
    continue
  fi

  # Check if phase is already completed in mega-progress.json
  PHASE_STATUS=$(jq -r --argjson p "$PHASE_NUM" \
    '(.phases[] | select(.phase == $p) | .status) // "pending"' \
    "$MEGA_PROGRESS" 2>/dev/null || echo "pending")

  if [[ "$PHASE_STATUS" == "completed" ]]; then
    echo "Skipping Phase $PHASE_NUM: $PHASE_TITLE (already completed)"
    continue
  fi

  echo ""
  echo "================================================================"
  echo "  Phase $PHASE_NUM of $TOTAL_PHASES: $PHASE_TITLE"
  echo "================================================================"
  echo ""

  BRANCH_NAME=$(make_branch_name "$PHASE_NUM" "$PHASE_TITLE")
  PREVIOUS_SUMMARY=$(get_previous_phases_summary "$PHASE_NUM")

  # Update progress: phase started
  update_progress_start "$PHASE_NUM" "$PHASE_TITLE" "$BRANCH_NAME"

  # Step 1: Generate PRD for this phase
  PRD_PATH=$(generate_phase_prd "$PHASE_NUM" "$PHASE_TITLE" "$PHASE_DESC" "$PREVIOUS_SUMMARY" "$PROJECT_NAME")
  if [[ $? -ne 0 || -z "$PRD_PATH" ]]; then
    echo "Error: Failed to generate PRD for Phase $PHASE_NUM"
    update_progress_failed "$PHASE_NUM" 0
    exit 1
  fi

  # Step 2: Convert PRD to prd.json
  convert_prd_to_json "$PRD_PATH" "$PHASE_NUM" "$PHASE_TITLE" "$PROJECT_NAME"
  if [[ $? -ne 0 ]]; then
    echo "Error: Failed to convert PRD for Phase $PHASE_NUM"
    update_progress_failed "$PHASE_NUM" 0
    exit 1
  fi

  # Step 3: Run ralph.sh to execute this phase
  echo ""
  echo "  Running ralph.sh for Phase $PHASE_NUM ..."
  echo ""

  RALPH_EXIT=0
  "$SCRIPT_DIR/ralph.sh" --tool "$TOOL" "$MAX_ITERATIONS" || RALPH_EXIT=$?

  # Determine how many iterations were used by checking prd.json
  STORIES_TOTAL=$(jq '.userStories | length' "$SCRIPT_DIR/prd.json" 2>/dev/null || echo "0")
  STORIES_DONE=$(jq '[.userStories[] | select(.passes == true)] | length' "$SCRIPT_DIR/prd.json" 2>/dev/null || echo "0")

  if [[ "$RALPH_EXIT" -eq 0 ]]; then
    echo ""
    echo "  Phase $PHASE_NUM completed! ($STORIES_DONE/$STORIES_TOTAL stories done)"

    # Update progress: phase completed
    update_progress_complete "$PHASE_NUM" "$STORIES_DONE"

    # Archive this phase
    archive_phase "$PHASE_NUM" "$PHASE_TITLE" "$BRANCH_NAME"

  else
    echo ""
    echo "  Phase $PHASE_NUM did not complete ($STORIES_DONE/$STORIES_TOTAL stories done)"
    echo "  ralph.sh exited with code $RALPH_EXIT"

    update_progress_failed "$PHASE_NUM" "$STORIES_DONE"

    echo ""
    echo "To resume, run:"
    echo "  ./mega-ralph.sh --plan $PLAN_FILE --start-phase $PHASE_NUM --tool $TOOL"
    exit 1
  fi
done

# ---------------------------------------------------------------------------
# All phases complete
# ---------------------------------------------------------------------------
echo ""
echo "================================================================"
echo "  MEGA-RALPH COMPLETE"
echo "================================================================"
COMPLETED_COUNT=$(jq '[.phases[] | select(.status == "completed")] | length' "$MEGA_PROGRESS")
echo "  All $COMPLETED_COUNT phases completed successfully!"
echo "  Progress: $MEGA_PROGRESS"
echo "================================================================"
