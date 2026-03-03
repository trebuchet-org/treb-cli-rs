---
name: masterplan
description: "Plan a large multi-phase project by doing deep codebase/domain discovery first, then generating a phased MASTER_PLAN.md. Use for rewrites, ports, greenfield apps, or any project too big for a single PRD. Triggers on: masterplan, plan project, rewrite in, port to, build from scratch, multi-phase plan, big project."
user-invocable: true
---

# Master Plan Generator

Plan large, multi-phase projects by first doing deep discovery, then breaking the work into 10-25 ordered phases suitable for mega-ralph execution.

---

## The Job

1. Receive a high-level project description from the user
2. **Discover**: deeply explore any reference codebases, domains, or existing code
3. Ask 3-7 clarifying questions (with lettered options)
4. Generate a phased `MASTER_PLAN.md`

**Important:** Do NOT implement anything. Do NOT create PRDs for individual phases. Just produce the master plan.

---

## When to Use This Skill

This skill is for projects that are **too big for a single PRD**:

- Rewriting a codebase in a different language
- Porting an app to a new framework
- Building a substantial application from scratch
- Major architectural overhauls spanning many modules
- Any project that will need multiple Ralph runs to complete

If the work fits in one PRD (3-8 user stories), use the `/prd` skill instead.

---

## Step 1: Understand the Scope

Before doing anything, classify the project:

| Type | What to discover |
|------|-----------------|
| **Rewrite/Port** | The source codebase — every feature, command, model, integration |
| **Greenfield** | The domain — similar tools, user workflows, core concepts |
| **Major Extension** | The existing codebase — architecture, patterns, extension points |

Ask the user:
- Where is the reference codebase (if any)?
- What is the target language/framework/stack?
- Is this a full rewrite or selective port?

---

## Step 2: Deep Discovery

This is the critical step that separates `/masterplan` from `/prd`. You must **thoroughly explore** before you can plan.

### For Rewrites/Ports (reference codebase exists):

Explore the source codebase and build a complete inventory:

1. **Commands / Entry Points** — Every CLI command, API endpoint, or UI route. List all of them with brief descriptions.
2. **Data Models** — Every struct, type, model, schema. Note relationships.
3. **External Integrations** — APIs, databases, file formats, subprocesses, network services.
4. **Configuration System** — Config files, env vars, CLI flags, defaults.
5. **Authentication / Secrets** — How keys, credentials, and permissions work.
6. **Internal Architecture** — Module boundaries, dependency injection, layering (domain/usecase/adapter).
7. **Testing Approach** — Unit tests, integration tests, fixtures, mocks.
8. **Build / Deploy** — Build system, CI, release packaging, installation.

**Be exhaustive.** Read READMEs, main entry points, every command file, config parsers, data model definitions, test files. A master plan based on incomplete discovery will produce broken phases.

### For Greenfield Projects (no reference codebase):

Research the domain and build a feature inventory:

1. **Core Workflows** — What are the 3-5 main things the user does?
2. **Data Model** — What entities exist? What are their relationships?
3. **Integrations** — What external services, APIs, or data sources are needed?
4. **User Interface** — CLI? Web? Desktop? What are the main screens/commands?
5. **Similar Tools** — What existing tools solve adjacent problems? What can we learn from them?

### For Major Extensions:

Explore the existing codebase like a rewrite, but focus on:

1. **Extension Points** — Where does new code hook in?
2. **Patterns** — What conventions does the codebase follow?
3. **Constraints** — What can't change? What's load-bearing?

### Discovery Output

After exploration, write a concise **Discovery Summary** (for yourself, not saved to a file) listing:
- Total scope: N commands, M models, K integrations
- Complexity hotspots: what's hardest to implement
- Risk areas: what might cause problems

---

## Step 3: Clarifying Questions

Ask 3-7 critical questions, formatted with lettered options for quick response:

```
1. What is the target technology stack?
   A. Rust with clap + tokio + serde
   B. TypeScript with Node.js
   C. Python with Click
   D. Other: [please specify]

2. How faithful should the rewrite be?
   A. Exact feature parity — every command, every flag
   B. Core features only — drop rarely-used commands
   C. Improved design — same goals, better architecture
   D. Other: [please specify]

3. What is the priority order for implementation?
   A. Foundation first, then commands in dependency order
   B. MVP subset first, then fill in remaining features
   C. Most-used commands first, then edge cases
   D. Other: [please specify]

4. How should the project handle testing?
   A. Comprehensive tests from the start (unit + integration per phase)
   B. Integration tests only (test the CLI end-to-end)
   C. Tests at the end (Phase N: add test suite)
   D. Other: [please specify]
```

Focus questions on decisions that **change the shape of the plan** — technology choices, scope boundaries, priority order, testing strategy. Skip questions where there's an obvious right answer.

---

## Step 4: Phase Design

Break the project into **10-25 phases** following these principles:

### Phase Ordering Rules

1. **Foundations first**: types, config, data layer before any commands or features
2. **Dependency order**: if Phase B uses code from Phase A, A comes first
3. **Simple before complex**: basic commands before advanced workflows
4. **Core before polish**: functionality before TUI, packaging, and release

### Typical Phase Structure (adapt to project)

```
Tier 1 — Foundation (Phases 1-4)
  Repository setup, core types, configuration, data/storage layer

Tier 2 — Engine (Phases 5-8)
  Core business logic, integrations, processing pipelines

Tier 3 — Basic Features (Phases 9-12)
  Simple commands/routes/endpoints that validate the architecture

Tier 4 — Advanced Features (Phases 13-17)
  Complex commands, orchestration, external service integrations

Tier 5 — Polish & Release (Phases 18-20+)
  UX polish, packaging, CI/CD, documentation
```

### Phase Sizing

Each phase becomes a Ralph PRD with **3-8 user stories**. Size phases so that:

- Each phase is **completable in 5-15 Ralph iterations**
- Each phase produces **something testable** (not just scaffolding)
- Each phase **builds on previous phases** without breaking them
- No phase requires knowledge of a later phase

### What Each Phase Entry Needs

For every phase, provide:

- **Phase number and title**
- **Description**: 2-4 sentences explaining what this phase accomplishes and why
- **Deliverables**: Bullet list of concrete outputs (files, features, APIs)
- **User stories estimate**: How many stories (3-8)
- **Dependencies**: Which previous phases it requires

---

## Step 5: Generate MASTER_PLAN.md

### File Format

```markdown
# Master Plan: [Project Name]

[1-2 paragraph overview: what we're building, why, target stack]

---

## Phase 1 -- [Title]

[2-4 sentence description]

**Deliverables**
- Concrete output 1
- Concrete output 2
- ...

**User stories:** N
**Dependencies:** none | Phase X, Y

---

## Phase 2 -- [Title]

...

---

## Dependency Graph (ASCII)

[Simple ASCII art showing phase dependencies]

---

## Summary Table

| Phase | Title | Stories | Depends On |
|------:|-------|--------:|------------|
| 1 | ... | N | -- |
| 2 | ... | N | 1 |
| ... | | | |
| **Total** | | **N** | |
```

### Important Format Notes

- Use `## Phase N -- Title` format (double-dash separator)
- This format is parsed by `mega-ralph.sh` — don't change the heading pattern
- Keep descriptions concise but complete enough for an AI to generate a detailed PRD from
- Include a dependency graph and summary table at the end

---

## Setup Check

Before generating the plan, verify the project is set up for mega-ralph:

1. Check if a `ralph/` directory exists in the project root
2. If it does **not** exist, tell the user to run the installer first:

```bash
curl -sL https://raw.githubusercontent.com/mento-protocol/mega-ralph/main/install.sh | bash -s -- --mega
```

3. If `ralph/` exists but `.ralph/mega-ralph.sh` is missing, tell the user to add mega-ralph support:

```bash
curl -sL https://raw.githubusercontent.com/mento-protocol/mega-ralph/main/install.sh | bash -s -- --mega
```

4. Only proceed with plan generation once the setup is confirmed.

---

## Output

- **Format:** Markdown (`.md`)
- **Location:** `ralph/` directory (root of the ralph install)
- **Filename:** `MASTER_PLAN.md`

---

## Example: Rewriting a Go CLI in Rust

User says: "Rewrite treb-cli (Go) in Rust"

**Discovery** reveals: 18 commands, 6 data models, 5 external integrations, 188 Go files.

**Plan** produces 20 phases:

| Tier | Phases | Focus |
|------|--------|-------|
| Foundation | 1-4 | Repo scaffold, domain types, config parsing, registry storage |
| Engine | 5-8 | Subprocess integration, sender system, event parsing, recording pipeline |
| Basic Commands | 9-12 | version, networks, init, config, list, show, run |
| Advanced Commands | 13-16 | verify, tag, register, sync, gen, compose |
| Polish | 17-20 | Safe integration, fork mode, housekeeping, TUI and release |

Each phase has 4-8 stories. Total: ~117 stories across 20 phases.

---

## Example: Greenfield SaaS App

User says: "Build a project management tool with Rust backend and React frontend"

**Discovery** identifies: 4 core workflows (projects, tasks, users, reporting), 6 data models, 3 integrations (auth, email, storage).

**Plan** produces 14 phases:

| Tier | Phases | Focus |
|------|--------|-------|
| Foundation | 1-3 | Monorepo setup, database schema, auth system |
| Backend | 4-7 | REST API for each domain (projects, tasks, users, reporting) |
| Frontend | 8-11 | Layout shell, project views, task board, user management |
| Integration | 12-13 | Email notifications, file attachments |
| Polish | 14 | Performance, testing, deployment |

---

## Checklist

Before saving MASTER_PLAN.md:

- [ ] Completed thorough discovery (read the source codebase / researched the domain)
- [ ] Asked clarifying questions about technology, scope, and priorities
- [ ] Phases are ordered by dependency (foundations first)
- [ ] Each phase has 3-8 user stories worth of work
- [ ] Each phase produces something testable
- [ ] No phase depends on a later phase
- [ ] Included dependency graph and summary table
- [ ] Used `## Phase N -- Title` heading format (parsed by mega-ralph.sh)
- [ ] Saved to `ralph/MASTER_PLAN.md`

---

## Anti-Patterns

**Don't do these:**

- **Planning without discovery**: "I'll figure out the details during implementation" — No. Incomplete discovery means broken phases that depend on things you didn't know existed.
- **Phases too big**: "Phase 3: Implement all 12 API endpoints" — Split into 2-3 phases of 4-6 endpoints each.
- **Phases too small**: "Phase 7: Add --verbose flag" — Combine with related work into a meaningful phase.
- **Wrong order**: UI phases before the backend they depend on. Always check: does this phase use anything from a later phase?
- **Vague deliverables**: "Phase 5: Core logic" — What specifically? Name the modules, the functions, the features.
- **Skipping the dependency graph**: The graph catches ordering mistakes that are easy to miss in a long list.
