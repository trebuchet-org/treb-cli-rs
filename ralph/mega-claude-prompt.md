You are generating a Product Requirements Document (PRD) for a single phase of a multi-phase project.

## Project: {{PROJECT_NAME}}

## Phase {{PHASE_NUMBER}}: {{PHASE_TITLE}}

## Master Plan

The following is the full master plan for this project. Read it to understand the overall vision and how this phase fits in, but ONLY generate a PRD for Phase {{PHASE_NUMBER}}.

---

{{MASTER_PLAN}}

---

## This Phase's Description

{{PHASE_DESCRIPTION}}

## What Was Built in Previous Phases

{{PREVIOUS_PHASES_SUMMARY}}

## Your Task

Generate a focused, well-scoped PRD for **Phase {{PHASE_NUMBER}}: {{PHASE_TITLE}}** only.

### PRD Requirements

1. **Title:** "PRD: Phase {{PHASE_NUMBER}} - {{PHASE_TITLE}}"

2. **Introduction:** Brief description of what this phase accomplishes and how it fits into the larger project.

3. **Goals:** 3-5 specific, measurable objectives for this phase.

4. **User Stories:** Each story must be:
   - Small enough to complete in ONE Ralph iteration (one context window, one focused change)
   - Ordered by dependency (schema/data first, then backend logic, then UI)
   - Assigned a sequential ID (US-001, US-002, etc.)
   - Include verifiable acceptance criteria (not vague)
   - Include "Typecheck passes" or equivalent quality check as a criterion
   - For UI stories: include "Verify in browser using dev-browser skill"

5. **Functional Requirements:** Numbered list (FR-1, FR-2, etc.)

6. **Non-Goals:** What this phase will NOT include.

7. **Technical Considerations:** Dependencies, constraints, integration points.

### Story Sizing Guidelines

**Right-sized stories (aim for these):**
- Add a database table/column and migration
- Create a single API endpoint or server action
- Build one UI component
- Add a configuration file or setup step
- Write tests for one module

**Too big (split these):**
- "Build the entire X feature" - split into schema, backend, UI pieces
- "Set up the full project" - split into init, config, first component, etc.
- "Implement authentication" - split into schema, middleware, UI, session

**Rule of thumb:** If you cannot describe the change in 2-3 sentences, it is too big.

### Phase Context

- This is Phase {{PHASE_NUMBER}} of a multi-phase project
- Previous phases have already been completed (see above)
- Your stories should build on what exists, not recreate it
- Reference existing code/patterns from previous phases when relevant

### Output

Save the PRD as a markdown file at:
```
ralph/tasks/prd-phase-{{PHASE_NUMBER}}-{{PHASE_TITLE}}.md
```

Use the phase title in kebab-case for the filename. For example, if the phase is "Project Setup", save to `ralph/tasks/prd-phase-01-project-setup.md`.

**Important:** Only generate the PRD file. Do NOT implement anything. Do NOT create prd.json. Do NOT make code changes.
