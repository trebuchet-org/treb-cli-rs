You are reviewing the results of a completed phase and updating the master plan with learnings.

## Project: {{PROJECT_NAME}}

## Just Completed: Phase {{PHASE_NUMBER}} - {{PHASE_TITLE}}

## Progress & Learnings From This Phase

The following is the progress log from the phase that just completed. It contains what was implemented, files changed, and critically — **learnings** discovered during implementation.

---

{{PHASE_PROGRESS}}

---

## Current Master Plan

The following is the current master plan. You will update it with learnings from the completed phase.

---

{{MASTER_PLAN}}

---

## Your Task

Review the learnings from Phase {{PHASE_NUMBER}} and update `{{PLAN_PATH}}` with relevant insights that will help **future phases** succeed.

### What to Update

1. **Add a "Learnings" section** under the completed phase's heading (if not already present):
   ```markdown
   ## Phase N -- Title

   [existing description...]

   **Learnings from implementation:**
   - Key insight 1 (e.g., "The codebase uses X pattern for Y")
   - Key insight 2 (e.g., "Z library requires config before use")
   - Gotcha discovered (e.g., "Don't forget to update A when changing B")
   ```

2. **Update future phase descriptions** if learnings reveal:
   - A dependency that wasn't anticipated
   - A better ordering for upcoming work
   - Technical constraints that affect scope
   - Patterns that future phases should follow

3. **Add to Architecture & Design Decisions** if the phase established patterns that all future phases should follow.

### Rules

- **DO NOT** change the `## Phase N -- Title` heading format (mega-ralph.sh parses these)
- **DO NOT** remove or rewrite existing phase descriptions — only append learnings
- **DO NOT** mark phases as done/skipped in the plan text
- **DO NOT** modify phases that are already completed (before Phase {{PHASE_NUMBER}})
- **DO** keep learnings concise — bullet points, not paragraphs
- **DO** focus on insights that are actionable for future phases
- If there are no meaningful learnings to add, make no changes

### Output

Update the file at `{{PLAN_PATH}}` directly. Only modify the master plan file — do not create any other files or make any code changes.
