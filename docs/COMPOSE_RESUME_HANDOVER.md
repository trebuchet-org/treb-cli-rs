# Compose Resume Redesign — Handover

## Branch
`feat/compose-resume-redesign` — started from main, has one commit: `compose_plan.rs` module.

## What's Done
- `crates/treb-forge/src/pipeline/compose_plan.rs` — new module with types, save/load, 6 passing tests
- Registered in `pipeline/mod.rs`
- Memory plan at `~/.claude/projects/-home-workspace-projects-treb-cli-rs/memory/project_compose_resume_redesign.md`

## What Needs Doing

### 1. Wire compose plan into `compose.rs`

**File**: `crates/treb-cli/src/commands/compose.rs`

Replace the old resume system (lines 822-841) with compose plan:

```rust
// OLD (remove):
let (skip_set, resumed_deployments) = if resume {
    if let Some(state) = load_compose_state()? { ... }
} else {
    delete_compose_state();
    ...
};

// NEW:
let plan = if resume {
    if let Some(existing) = compose_plan::load_plan(&cwd, &file, chain_id) {
        if !compose_plan::plan_matches_compose(&existing, &compose_hash) && !json {
            eprintln!("Warning: compose file changed since last run; resuming anyway");
        }
        Some(existing)
    } else {
        None
    }
} else {
    None
};
// Build skip_set from plan's completed components
let skip_set: HashSet<String> = plan.as_ref()
    .map(|p| p.components.iter()
        .filter(|c| c.status == ComponentStatus::Broadcast)
        .map(|c| c.name.clone())
        .collect())
    .unwrap_or_default();
```

**Create plan at start of broadcast** (around line 1095):
```rust
let components_for_plan: Vec<(String, String)> = components_to_run.iter()
    .map(|name| (name.clone(), compose.components[name].script.clone()))
    .collect();
let mut plan = compose_plan::create_plan(&file, &compose_hash, chain_id, &components_for_plan);
```

**Update plan after each component broadcasts** — in the results loop (around line 1147):
```rust
compose_plan::update_component(
    &mut plan,
    &sr.name,
    ComponentStatus::Broadcast,
    Some(broadcast_file_path),
    deferred_file_path,
);
compose_plan::save_plan(&cwd, &plan)?;
```

**Delete plan on success** (around line 1195):
```rust
// On success: keep the plan file (it's useful as a record)
// On failure: plan stays with partial status for resume
```

### 2. Remove old state files

**Delete from `compose.rs`**:
- `COMPOSE_STATE_FILE` constant (line 131)
- `ComposeState` struct (lines 134-143)
- `compute_file_hash` — move to compose_plan.rs or keep as utility
- `load_compose_state()` function
- `save_compose_state()` function
- `delete_compose_state()` function

**Delete from `broadcast_writer.rs`**:
- `SESSION_STATE_FILE` constant (line 685)
- `load_session_state()` (line 690)
- `save_session_state()` (line 700)
- `delete_session_state()` (line 710)

**Delete from `types.rs`**:
- `SessionState` struct
- `ScriptProgress` struct
- `ScriptPhase` enum

**Update `orchestrator.rs`**:
- `SimulatedSession` struct — remove `state_scripts`, `config_hash` fields
- `simulate_all()` — remove all `save_session_state` / `load_session_state` calls
- `broadcast_all()` — remove all `save_session_state` calls and `state_scripts` tracking
- The session state was used to skip already-broadcast scripts on resume — this is now handled by the compose plan's component status

### 3. Update `broadcast_all()` in orchestrator.rs

The `broadcast_all` method (line 1806) currently:
1. Loads session state for resume
2. Saves session state after each script
3. Deletes session state on completion

Replace with:
1. Accept a callback or return per-component status that compose.rs uses to update the plan
2. No internal state file management — that's compose.rs's job

**Option A** (simpler): `broadcast_all` returns `Vec<ScriptResult>` as before, compose.rs updates the plan after.

**Option B** (cleaner): `broadcast_all` takes a `on_component_complete` callback that compose.rs uses to save the plan.

I'd go with Option A — minimal change to broadcast_all.

### 4. Resume from broadcast files

The `--resume` flag on compose should:
1. Load compose plan → get list of pending components
2. For each pending component:
   a. Check if `broadcast_file` exists → load `ResumeState` from it
   b. If no broadcast file → run from scratch (simulate + broadcast)
   c. If broadcast file exists → use `route_all_with_resume()` to resume mid-broadcast

The `load_resume_state()` function in `broadcast_writer.rs` already does the heavy lifting — reads the broadcast file, polls on-chain receipts, classifies txs as confirmed/pending/unsent.

### 5. Execution ordering

The plan enforces: for each script in dependency order, immediates first, deferred queued at end.

Current flow already does this in `broadcast_all()`:
- Wallet txs are broadcast immediately
- Safe/Governor proposals are deferred (collected by merge logic)
- Phase C submits merged proposals at the end

No change needed for the ordering — it's already correct.

## Key File Locations

| File | What to change |
|------|---------------|
| `crates/treb-cli/src/commands/compose.rs` | Replace ComposeState with compose plan, remove old state functions |
| `crates/treb-forge/src/pipeline/orchestrator.rs` | Remove SessionState tracking from simulate_all/broadcast_all |
| `crates/treb-forge/src/pipeline/broadcast_writer.rs` | Remove session state functions (keep ResumeState/load_resume_state) |
| `crates/treb-forge/src/pipeline/types.rs` | Remove SessionState/ScriptProgress/ScriptPhase types |
| `crates/treb-forge/src/pipeline/compose_plan.rs` | Already done — the new module |

## Types to Keep vs Remove

**Keep**:
- `ResumeState` in broadcast_writer.rs — used by routing for per-tx resume
- `load_resume_state()` — reads broadcast files for resume
- `ScriptResult`, `PipelineResult` — unchanged

**Remove**:
- `SessionState`, `ScriptProgress`, `ScriptPhase` — replaced by compose plan
- `ComposeState` — replaced by compose plan
- `save_session_state`, `load_session_state`, `delete_session_state`
- `save_compose_state`, `load_compose_state`, `delete_compose_state`

## Test Impact

- `compose.rs` tests that reference `ComposeState` need updating
- `orchestrator.rs` tests that use `SessionState` / `ScriptPhase` need updating
- E2E compose tests with `--resume` should still pass (same behavior, different internal state)
- New test: verify compose plan file is written and used for resume

## Current State of Branch

```
main ← feat/compose-resume-redesign (1 commit ahead)
         └── compose_plan.rs module added
```

All tests pass on the branch. Ready to wire into compose.rs.
