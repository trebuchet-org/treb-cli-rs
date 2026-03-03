You are converting a PRD (Product Requirements Document) into Ralph's prd.json format.

## Input

Read the PRD file at: `{{PRD_FILE}}`

## Project: {{PROJECT_NAME}}

## Phase: {{PHASE_NUMBER}} - {{PHASE_TITLE}}

## Output Format

Create a `ralph/.state/prd.json` file with this exact structure:

```json
{
  "project": "{{PROJECT_NAME}}",
  "branchName": "ralph/phase-{{PHASE_NUMBER}}-{{PHASE_TITLE}}",
  "description": "[Description from PRD title/intro]",
  "userStories": [
    {
      "id": "P{{PHASE_NUMBER}}-US-001",
      "title": "[Story title]",
      "description": "As a [user], I want [feature] so that [benefit]",
      "acceptanceCriteria": [
        "Criterion 1",
        "Criterion 2",
        "Typecheck passes"
      ],
      "priority": 1,
      "passes": false,
      "notes": ""
    }
  ]
}
```

## Branch Naming

Use this format for the branch name: `ralph/phase-{{PHASE_NUMBER}}-{{PHASE_TITLE}}`

Convert the phase title to kebab-case (lowercase, hyphens for spaces, no special characters). Pad the phase number to 2 digits.

For example: `ralph/phase-03-api-endpoints`

## Conversion Rules

1. **Each user story in the PRD becomes one JSON entry** - do not merge or split unless a story is clearly too large for one iteration.

2. **IDs:** Phase-prefixed, sequential: `P{{PHASE_NUMBER}}-US-001`, `P{{PHASE_NUMBER}}-US-002`, etc.

3. **Priority:** Matches the order from the PRD (first story = priority 1). Stories must be ordered by dependency: schema/data first, then backend, then UI.

4. **All stories start with:** `"passes": false` and `"notes": ""`

5. **Acceptance criteria must be verifiable** - convert any vague criteria to specific, checkable ones.

6. **Always include** "Typecheck passes" (or equivalent quality check) in every story's acceptance criteria.

7. **For UI stories**, include "Verify in browser using dev-browser skill" in acceptance criteria.

## Story Size Validation

Before writing prd.json, verify each story is small enough for one Ralph iteration:
- Can be described in 2-3 sentences
- Touches a focused set of files
- Has 3-6 acceptance criteria

If any story seems too large, split it into multiple smaller stories.

## Your Task

1. Read the PRD at `{{PRD_FILE}}`
2. Convert it to prd.json following the rules above
3. Write the result to `ralph/.state/prd.json`
4. Validate the JSON is well-formed

**Important:** Only create prd.json. Do NOT implement any code. Do NOT modify the PRD. Do NOT run ralph.sh.
