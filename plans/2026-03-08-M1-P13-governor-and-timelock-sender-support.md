# PRD: Phase 13 - Governor and Timelock Sender Support

## Introduction

This phase completes the OpenZeppelin Governor account type support end-to-end. The foundational infrastructure already exists: `GovernorProposal` type, `GovernorProposalStore` (persisting to `governor-txs.json`), `ResolvedSender::Governor` variant, `GovernorProposalCreated` event handling, `hydrate_governor_proposals()`, and registry insertion in the pipeline orchestrator. What remains is (1) surfacing governor proposals in the `treb run` command output (both human and JSON), (2) building an on-chain Governor contract state polling client, (3) extending the `treb sync` command to poll governor proposal status updates, and (4) adding golden file test coverage for the governor flow.

Phase 12 (Safe Multisig End-to-End) established the pattern for composite sender flows: the pipeline detects the sender type, processes sender-specific events, inserts records into a dedicated registry store, and the sync command polls an external service for status updates. This phase follows the same pattern but uses on-chain RPC calls (to the Governor contract's `state(proposalId)` function) instead of an external API service.

## Goals

1. **Governor proposal visibility in run output**: When `treb run` executes with a Governor sender, display proposal details (proposal ID, governor address, timelock address, status) in both human-readable tree output and `--json` output, matching the Safe transaction display pattern.

2. **On-chain governor state polling**: Build an RPC-based client that calls the Governor contract's `state(uint256)` view function to map on-chain proposal state to `ProposalStatus`, enabling the sync command to track proposal lifecycle (Pending -> Active -> Succeeded -> Queued -> Executed).

3. **Sync command governor extension**: Extend `treb sync` to poll governor proposals alongside Safe transactions, updating proposal status, execution details, and linked Transaction records when proposals reach Executed state.

4. **Consistent error handling**: Add a `Governor(String)` error variant to `TrebError` for governor-specific failures, following the `Safe(String)` pattern from Phase 12.

5. **Golden file test coverage**: Add golden file tests for the governor sender flow in `treb run` and governor proposal syncing in `treb sync`.

## User Stories

### P13-US-001: PipelineResult Governor Proposal Propagation and Error Variant

**Description:** Add governor proposal data to `PipelineResult` so the run command can display proposals, and add a `Governor(String)` error variant to `TrebError` for governor-specific error messages.

**Details:**
- Add `governor_proposals: Vec<GovernorProposal>` field to `PipelineResult` in `crates/treb-forge/src/pipeline/types.rs`
- Populate the field in the orchestrator (`crates/treb-forge/src/pipeline/orchestrator.rs`):
  - In the non-dry-run path: collect governor proposals into the result **before** inserting them into the registry (same records)
  - In the dry-run path: populate from hydrated proposals without writing to registry
- Add `#[error("governor error: {0}")] Governor(String)` variant to `TrebError` in `crates/treb-core/src/error.rs`, following the `Safe(String)` pattern
- Ensure the `PipelineResult` default construction in tests accounts for the new field (initialize to `vec![]`)

**Acceptance Criteria:**
- `PipelineResult` has a `governor_proposals` field of type `Vec<GovernorProposal>`
- Orchestrator populates `governor_proposals` in both live and dry-run paths
- `TrebError::Governor(String)` variant exists with Display impl
- `cargo check -p treb-forge` and `cargo check -p treb-core` pass
- Unit test: orchestrator test verifies governor proposals appear in PipelineResult when GovernorProposalCreated events are present

---

### P13-US-002: Run Command Governor Proposal Output

**Description:** Display governor proposals in the `treb run` command output when the deployer sender is a Governor type. Show proposal details in both human-readable and JSON formats, with governor-specific stage messages.

**Details:**
- **Human output** (in `display_result_human()` in `crates/treb-cli/src/commands/run.rs`):
  - After the deployment tree and transaction sections, add a "Governor Proposals" section when `result.governor_proposals` is non-empty
  - For each proposal, display:
    - Proposal ID (truncated if very long uint256)
    - Governor address (truncated via `output::truncate_address()`)
    - Timelock address (if present, truncated)
    - Status (from `ProposalStatus` Display impl)
    - Linked transaction count
  - Use `print_kv`-style formatting for each proposal's fields
  - In dry-run mode, use "would be proposed" language
- **JSON output** (in `display_result_json()`:
  - Add `governor_proposals: Vec<GovernorProposalJson>` field to `RunOutputJson`
  - `GovernorProposalJson` struct with: `proposal_id`, `governor_address`, `timelock_address`, `status`, `transaction_ids`
  - Use `#[serde(rename_all = "camelCase")]` for Go-compatible JSON
- **Stage messages**:
  - When governor sender is detected (check `deployer_sender.is_governor()` before pipeline execution), print governor-specific stage: `print_stage("\u{1f3db}", "Creating governance proposal...")` instead of "Broadcasting..."
  - The stage message goes to stderr and is suppressed in `--json` mode (follow existing `if !json` guards)
- **Verbose output**:
  - In verbose post-execution output, add governor proposal count: `"Governor proposals: N"` to the summary
  - Show governor/timelock addresses from resolved sender in pre-execution context
- **Summary line**: Include governor proposal count: `"1 deployment recorded, 1 governor proposal created"`

**Acceptance Criteria:**
- Human output shows governor proposals section with proposal ID, addresses, status
- JSON output includes `governorProposals` array with camelCase fields
- Governor-specific stage message appears when deployer is Governor sender
- Verbose mode shows governor context and proposal counts
- Dry-run uses "would be proposed" language
- `cargo check -p treb-cli` passes

---

### P13-US-003: Governor On-Chain State Polling Client

**Description:** Build an RPC-based client that queries a Governor contract's `state(uint256 proposalId)` view function to determine the current on-chain proposal state, and map the uint8 return value to `ProposalStatus`.

**Details:**
- Create a new module `crates/treb-forge/src/governor.rs` (or a minimal helper within the sync command) containing:
  - `async fn query_proposal_state(rpc_url: &str, governor_address: &str, proposal_id: &str) -> Result<ProposalStatus, TrebError>`
  - Uses `eth_call` JSON-RPC to call `state(uint256)` (selector: `0x3e4f49e6`) on the governor contract
  - The `proposal_id` is a uint256 string — encode as ABI-encoded uint256 parameter
  - Returns the uint8 result mapped to `ProposalStatus`:
    - 0 = Pending, 1 = Active, 2 = Canceled, 3 = Defeated, 4 = Succeeded, 5 = Queued, 6 = Expired (map to Defeated), 7 = Executed
  - Note: OZ Governor has an "Expired" state (6) that our `ProposalStatus` enum doesn't have — map Expired to Defeated since both represent terminal non-executed states
- Use `reqwest::Client` for the RPC call (already a dependency)
- The function takes an RPC URL (resolved from foundry.toml network config or the proposal's chain_id)
- Add a helper `fn resolve_rpc_for_chain_id(chain_id: u64, cwd: &Path) -> Option<String>` that looks up the RPC URL from foundry.toml's `[rpc_endpoints]` by matching chain IDs (try all endpoints, call `eth_chainId` on each, cache results) — alternatively, accept the RPC URL as a required `--rpc-url` flag on the sync command for governor proposals
- Error handling: wrap RPC failures in `TrebError::Governor`

**Acceptance Criteria:**
- `query_proposal_state()` function sends correct `eth_call` for `state(uint256)` to the Governor contract
- All 8 OZ proposal states (0-7) are mapped correctly to `ProposalStatus` variants
- RPC failures produce `TrebError::Governor` errors with descriptive messages
- `cargo check -p treb-forge` passes
- Unit test: verify ABI encoding of `state(uint256)` calldata for a sample proposal ID
- Unit test: verify mapping of uint8 return values 0-7 to expected ProposalStatus variants

---

### P13-US-004: Sync Command Governor Proposal Extension

**Description:** Extend the `treb sync` command to poll on-chain Governor contract state for tracked governor proposals, update their status in the registry, and update linked Transaction records when proposals reach Executed state.

**Details:**
- In `crates/treb-cli/src/commands/sync.rs`, after the existing Safe transaction sync block, add a governor proposal sync block:
  1. Load governor proposals from registry (optionally filtered by `--network` chain_id)
  2. Skip proposals already in terminal states (Executed, Canceled, Defeated)
  3. For each non-terminal proposal:
     - Resolve the RPC URL for the proposal's `chain_id` from foundry.toml `[rpc_endpoints]` (reuse `resolve_rpc_url_for_chain_id` from run.rs or extract to shared location)
     - Call `query_proposal_state(rpc_url, governor_address, proposal_id)` to get current on-chain state
     - If the state has changed, update the proposal in the registry
     - If the proposal transitioned to Executed, set `executed_at` to current time and update linked Transaction records (set status to Executed, similar to Safe transaction execution handling)
  4. If `--clean` is passed, remove proposals whose governor contract reverts on `state()` (contract may have been destroyed/upgraded)
- **Output formatting** (human):
  - Add "Governor proposals" to the stage messages: `print_stage("\u{1f50d}", "Syncing governor proposals...")`
  - Summary includes governor proposal counts alongside Safe transaction counts:
    - `Governor proposals synced: N`
    - `Updated: M` (styled with WARNING color)
    - `Newly executed: K` (styled with SUCCESS color)
  - If `--clean` is active, show removed count
- **JSON output**: Extend `SyncOutputJson` with governor-specific fields:
  - `governor_synced: usize`
  - `governor_updated: usize`
  - `governor_newly_executed: usize`
  - `governor_removed: usize` (when `--clean`)
- **Error handling**: Non-fatal — if RPC call fails for one proposal, log a warning and continue with the rest (follow the Safe sync error accumulation pattern)

**Acceptance Criteria:**
- `treb sync` polls on-chain state for non-terminal governor proposals
- Proposal status updates are persisted to `governor-txs.json`
- Linked Transaction records are updated when proposals become Executed
- `--clean` removes proposals whose governor contract is unreachable
- Human output shows governor sync summary with counts and colors
- JSON output includes governor sync fields with camelCase
- RPC failures for individual proposals don't abort the entire sync
- `cargo check -p treb-cli` passes

---

### P13-US-005: Golden File Tests for Governor Flow

**Description:** Add golden file test fixtures and test cases for the governor sender flow in `treb run` output and governor proposal syncing in `treb sync` output.

**Details:**
- **Run command golden files**:
  - Create test fixture with a governor sender configuration in the treb.toml fixture (type `oz_governor`, governor address, timelock address, proposer reference)
  - Create golden test `run_governor_proposal` that exercises the run command with a governor sender and verifies:
    - Governor-specific stage messages appear
    - Governor proposal section in human output (proposal ID, governor address, status)
    - Summary line includes governor proposal count
  - Create golden test `run_governor_proposal_json` that verifies `--json` output includes `governorProposals` array
  - Create golden test `run_governor_dry_run` that verifies dry-run with governor sender uses "would be proposed" language
- **Sync command golden files**:
  - Seed registry with governor proposals in various states (Pending, Active, Queued)
  - Create golden test `sync_governor` that verifies governor sync output
  - Create golden test `sync_governor_json` for JSON sync output with governor fields
- **Normalizers**: Add normalizers as needed for proposal IDs (large uint256 values) and timestamps
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli` to generate initial golden files
- Verify no regressions in existing golden tests

**Acceptance Criteria:**
- Golden tests for run with governor sender pass (human, JSON, dry-run)
- Golden tests for sync with governor proposals pass (human, JSON)
- No regressions in existing golden file tests (list, show, run, sync)
- `cargo test -p treb-cli` passes in full

## Functional Requirements

- **FR-1:** When the deployer sender is an `oz_governor` type, `treb run` displays governor-specific stage messages (e.g., "Creating governance proposal..." instead of "Broadcasting...").
- **FR-2:** `treb run` human output includes a "Governor Proposals" section showing proposal ID, governor address, timelock address (if configured), status, and linked transaction count.
- **FR-3:** `treb run --json` output includes a `governorProposals` array with camelCase field names matching the `GovernorProposal` domain type.
- **FR-4:** `treb sync` polls the on-chain Governor contract `state(uint256)` function for each tracked governor proposal and updates the registry when the state changes.
- **FR-5:** When a governor proposal transitions to Executed state during sync, linked Transaction records are updated to Executed status (matching the Safe execution update pattern).
- **FR-6:** `treb sync --clean` removes governor proposals whose Governor contract is unreachable (RPC reverts on `state()` call).
- **FR-7:** `treb sync` JSON output includes governor-specific counts (`governorSynced`, `governorUpdated`, `governorNewlyExecuted`, `governorRemoved`).
- **FR-8:** On-chain Governor proposal states 0-7 (Pending, Active, Canceled, Defeated, Succeeded, Queued, Expired, Executed) are correctly mapped to `ProposalStatus` enum variants, with Expired mapping to Defeated.
- **FR-9:** All governor-specific output respects `--json` mode (human output suppressed), `NO_COLOR`/`TERM=dumb`, and non-TTY detection via `color::is_color_enabled()`.
- **FR-10:** RPC failures during governor sync are non-fatal — individual proposal errors are accumulated and reported without aborting the sync.

## Non-Goals

- **No governance proposal submission.** The actual proposal transaction is created by the forge script execution (via TrebDeploy base contract). The CLI does not construct or submit `propose()` calldata to the Governor contract — that is handled by the Solidity script.
- **No voting support.** The CLI does not vote on proposals — it only tracks their status via on-chain state queries.
- **No timelock queue/execute transactions.** Timelock operations (queue, execute) are initiated externally or via separate scripts. The CLI tracks state changes passively through the sync command.
- **No Governor event indexing.** The sync command uses direct `state()` RPC calls, not event log filtering. Historical event indexing is out of scope.
- **No cross-governor batch proposals.** Each Governor proposal is tracked independently. Multi-governor orchestration is not supported.
- **No changes to the Safe multisig flow.** Phase 12's Safe integration is unchanged.
- **No interactive proposal management.** No CLI prompts for proposal actions (vote, queue, execute). The CLI is read-only after initial proposal creation via forge script.
- **No Governor contract deployment or configuration.** The CLI assumes the Governor and Timelock contracts are already deployed and configured.

## Technical Considerations

### Dependencies
- **Phase 5 (run command)**: Run command output patterns, stage display functions (`print_stage`, `print_warning_banner`, `eprint_kv`), JSON output structure (`RunOutputJson`)
- **Phase 12 (Safe multisig)**: Sync command patterns for polling external state, updating registry records, and cascading status changes to linked Transaction records
- **Phase 3 (output framework)**: `TreeNode`, `color::*`, `badge::*` for styled output
- **`reqwest`**: Already a dependency for RPC calls — reused for Governor contract `eth_call`
- **No new crate dependencies required**

### Key Files to Modify
- `crates/treb-core/src/error.rs` — Add `Governor(String)` variant
- `crates/treb-forge/src/pipeline/types.rs` — Add `governor_proposals` to `PipelineResult`
- `crates/treb-forge/src/pipeline/orchestrator.rs` — Populate `governor_proposals` in result
- `crates/treb-cli/src/commands/run.rs` — Governor proposal output section (human + JSON)
- `crates/treb-cli/src/commands/sync.rs` — Governor proposal sync block
- New: `crates/treb-forge/src/governor.rs` — On-chain state polling client

### Existing Infrastructure (Already Built)
- `GovernorProposal` type with camelCase serde (`crates/treb-core/src/types/governor_proposal.rs`)
- `ProposalStatus` enum: Pending, Active, Succeeded, Queued, Executed, Canceled, Defeated (`crates/treb-core/src/types/enums.rs`)
- `GovernorProposalStore` with CRUD + persistence to `governor-txs.json` (`crates/treb-registry/src/store/governor_proposals.rs`)
- `ResolvedSender::Governor` with `governor_address`, `timelock_address`, `proposer` (`crates/treb-forge/src/sender.rs`)
- `GovernorProposalCreated` event from treb-sol (`crates/treb-forge/src/events/abi.rs`)
- `hydrate_governor_proposals()` and `extract_governor_proposal_created()` (`crates/treb-forge/src/pipeline/hydration.rs`, `orchestrator.rs`)
- Registry facade methods: `insert_governor_proposal`, `update_governor_proposal`, `remove_governor_proposal`, `list_governor_proposals`, `get_governor_proposal`, `governor_proposal_count` (`crates/treb-registry/src/registry.rs`)
- Governor sender resolution with circular reference detection (`crates/treb-forge/src/sender.rs`)

### ABI Encoding for state(uint256)
- Function selector: `keccak256("state(uint256)")[:4]` = `0x3e4f49e6`
- Parameter encoding: proposal_id as ABI-encoded uint256 (32 bytes, zero-padded)
- Return value: single uint8 (0-7) representing `IGovernor.ProposalState`
- The proposal_id stored in `GovernorProposal.proposal_id` is a decimal string representation of a uint256 — parse with `U256::from_dec_str()` before ABI encoding

### RPC URL Resolution for Sync
- Governor proposals store `chain_id` but not `rpc_url`
- The sync command needs to resolve an RPC URL for each chain_id
- Reuse `resolve_rpc_url_for_chain_id()` from `run.rs` (currently crate-private with `pub(crate)`) — make it available to sync.rs or extract to a shared module
- Alternatively, require `--rpc-url` flag for governor sync (simpler but less ergonomic)
- Best approach: iterate foundry.toml `[rpc_endpoints]`, match by calling `eth_chainId` on each, cache the mapping

### Golden File Patterns
- Use `UPDATE_GOLDEN=1 cargo test -p treb-cli -- <test_name>` for auto-regeneration
- Add normalizers for governor-specific content (proposal IDs, timestamps) if they vary between runs
- Governor golden tests need fixture data: governor proposals in `governor-txs.json` test fixture, governor sender config in `treb.toml` fixture
- Follow the pattern from Phase 12's Safe golden tests for sync command fixtures

### Patterns to Follow
- Safe sync flow in `sync.rs`: group by (address, chain_id), poll external state, diff against local, accumulate errors, output summary — follow exactly for governor
- `RecordedDeployment.safe_transaction` pattern: consider adding `governor_proposal: Option<GovernorProposal>` to `RecordedDeployment` for linking proposals to deployments (but only if needed for display — may not be necessary since proposals are separate entities)
- `styled()` helper for conditional color (from show.rs/run.rs)
- `print_stage()` for progress indicators to stderr
- `output::print_json()` for sorted-key JSON output
