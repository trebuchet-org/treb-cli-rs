# PRD: Phase 7 - Deploy Real Governor Stack on Anvil

## Introduction

Phase 7 deploys a real OpenZeppelin-style governance stack (GovernanceToken + TrebTimelock + TrebGovernor) on the test Anvil fixture, then exercises the full governor routing pipeline through integration tests. This mirrors the Safe infrastructure established in Phase 6, extending coverage to the governor → timelock → execution path and the recursive governor → proposer routing chain.

With Phase 6 complete, all Safe routing paths have integration test coverage. Phase 7 closes the remaining gap: governor-based deployments where `fork_routing.rs::execute_governor_on_fork()` schedules through a timelock, warps time, and executes — all against real Solidity contracts instead of impersonation-only stubs.

## Goals

1. **Functional governor contracts on Anvil**: GovernanceToken, TrebTimelock, and TrebGovernor stubs that support the exact ABI surface used by `fork_routing.rs` — specifically `getMinDelay()`, `scheduleBatch()`, `executeBatch()`, `grantRole()`, and OZ `AccessControl` role checks.

2. **Reusable deployment helper**: A `deploy_governor()` Rust e2e helper (mirroring `deploy_safe()`) that deploys the full stack via `forge script` and returns typed addresses for use in tests.

3. **Governor → Wallet propose() coverage**: Integration test proving the governor routing pipeline compiles a `propose()` calldata, routes it through a wallet proposer, and records a `GovernorProposal` in `governor-txs.json`.

4. **Governor → Safe(1/1) chain coverage**: Integration test proving recursive routing where a governor's proposer is a Safe(1/1), exercising the depth-2 routing path.

5. **Fork simulation coverage**: Integration test exercising `execute_governor_on_fork()` with the timelock schedule → warp → execute path against real contracts, verifying on-chain state changes.

## User Stories

### P7-US-001: Add minimal governance contract stubs to test fixture

Create `GovernanceToken.sol`, `TrebTimelock.sol`, and `TrebGovernor.sol` in `crates/treb-cli/tests/fixtures/project/src/governance/`.

**GovernanceToken** — Minimal ERC20 with:
- `mint(address to, uint256 amount)` (public, no access control — test-only)
- `delegate(address delegatee)` to enable voting power
- `getVotes(address account) returns (uint256)` for governor quorum checks
- Standard `balanceOf`, `transfer`, `totalSupply`

**TrebTimelock** — Minimal OZ-compatible TimelockController:
- Storage: `minDelay`, role mappings for `PROPOSER_ROLE`, `EXECUTOR_ROLE`, `DEFAULT_ADMIN_ROLE`
- `constructor(uint256 minDelay, address[] proposers, address[] executors, address admin)`
- `getMinDelay() returns (uint256)` — called by `fork_routing.rs`
- `scheduleBatch(address[], uint256[], bytes[], bytes32, bytes32, uint256)` — called by `fork_routing.rs`
- `executeBatch(address[], uint256[], bytes[], bytes32, bytes32)` — called by `fork_routing.rs`
- `grantRole(bytes32, address)` / `hasRole(bytes32, address)` — called by `fork_routing.rs`
- Operation ID computation: `keccak256(abi.encode(targets, values, payloads, predecessor, salt))`
- Operation state tracking: `Unset → Waiting → Ready → Done` based on timestamps

**TrebGovernor** — Minimal Governor stub:
- `propose(address[], uint256[], bytes[], string) returns (uint256)` — the ABI called by `encode_governor_propose()` (selector `0x7d5e81e2`)
- `state(uint256 proposalId) returns (uint8)` — called by `governor.rs::query_proposal_state()`
- `hashProposal(address[], uint256[], bytes[], bytes32) returns (uint256)` — proposal ID computation
- Store proposal metadata (targets, values, calldatas, proposer) on `propose()`
- Constructor takes token address and timelock address

All contracts use `pragma solidity =0.8.30;` matching the existing Safe stubs.

**Acceptance criteria:**
- Three `.sol` files exist in `tests/fixtures/project/src/governance/`
- `forge build` compiles successfully in the fixture project
- Timelock's `scheduleBatch` / `executeBatch` signature matches the `sol!` ABI in `fork_routing.rs`
- Governor's `propose()` selector matches `0x7d5e81e2`
- Governor's `state()` returns uint8 values (0–7) matching `ProposalStatus` mapping in `governor.rs::map_onchain_state()`

---

### P7-US-002: Create DeployGovernance.s.sol Forge deployment script

Create `crates/treb-cli/tests/fixtures/project/script/DeployGovernance.s.sol` that deploys the full governance stack on Anvil.

**Environment variables:**
- `GOV_TOKEN_SUPPLY` — initial token supply to mint to deployer (required)
- `GOV_TIMELOCK_DELAY` — minimum timelock delay in seconds (required)
- `GOV_PROPOSER` — address that gets PROPOSER_ROLE on the timelock (required; the governor address is not known until deployment, so the script must grant the role post-deploy)

**Deployment sequence:**
1. Deploy `GovernanceToken` via `new`
2. Mint `GOV_TOKEN_SUPPLY` to `msg.sender`
3. `msg.sender` delegates to self (activates voting power)
4. Deploy `TrebTimelock` with `minDelay`, empty proposer/executor arrays, `msg.sender` as admin
5. Deploy `TrebGovernor` with token and timelock addresses
6. Grant `PROPOSER_ROLE` to the governor on the timelock
7. Grant `EXECUTOR_ROLE` to `address(0)` on the timelock (open executor, matches OZ defaults)

**Emits** a `GovernanceInfraDeployed` event with all three addresses and config values for artifact parsing.

Uses `vm.startBroadcast()` / `vm.stopBroadcast()`.

**Acceptance criteria:**
- `forge script script/DeployGovernance.s.sol --broadcast --rpc-url <anvil>` succeeds
- Broadcast artifacts in `broadcast/DeployGovernance.s.sol/<chain_id>/run-latest.json` contain CREATE entries for all three contracts
- After deployment, `cast call <timelock> "getMinDelay()(uint256)"` returns the configured delay
- After deployment, `cast call <timelock> "hasRole(bytes32,address)(bool)" <PROPOSER_ROLE_HASH> <governor>` returns `true`

---

### P7-US-003: Add Rust e2e helper for Governor deployment on Anvil

Create `crates/treb-cli/tests/e2e/deploy_governor.rs` mirroring `deploy_safe.rs`.

**Public API:**
```rust
pub struct DeployedGovernor {
    pub governor_address: Address,
    pub timelock_address: Address,
    pub token_address: Address,
    pub timelock_delay: u64,
}

pub fn deploy_governor(
    project_dir: &Path,
    rpc_url: &str,
    timelock_delay: u64,
    token_supply: u64,
) -> DeployedGovernor;

pub fn verify_governor_via_eth_call(
    rpc_url: &str,
    gov: &DeployedGovernor,
);
```

**Implementation:**
1. Run `forge script script/DeployGovernance.s.sol --broadcast --rpc-url <url> --private-key <anvil_key_0>` with environment variables
2. Parse `broadcast/DeployGovernance.s.sol/<chain_id>/run-latest.json` to extract CREATE addresses for `GovernanceToken`, `TrebTimelock`, `TrebGovernor`
3. `verify_governor_via_eth_call` uses `cast call` to verify: `getMinDelay()` matches, governor has `PROPOSER_ROLE` on timelock

**Also add a standalone e2e test** in a new `crates/treb-cli/tests/e2e_governor_deploy.rs` file (mirroring `e2e_safe_deploy.rs`):
- `governor_deploy_and_verify()`: spawn Anvil → deploy governor stack → verify via eth_call

Register `deploy_governor` module in `crates/treb-cli/tests/e2e/mod.rs`.

**Acceptance criteria:**
- `deploy_governor()` returns valid non-zero addresses for all three contracts
- `verify_governor_via_eth_call()` passes without assertion failures
- `e2e_governor_deploy.rs` test passes with `cargo test -p treb-cli --test e2e_governor_deploy`
- Helper is blocking (designed for `tokio::task::spawn_blocking`)

---

### P7-US-004: Create deployment-through-Governor Forge script

Create `crates/treb-cli/tests/fixtures/project/script/DeployViaGovernor.s.sol` — a Forge script that deploys a contract (Counter) through a governor address, triggering treb's governor routing pipeline.

**Pattern:** Mirror `DeployViaSafe.s.sol` but with governor routing signals.

**Environment variables:**
- `GOVERNOR_ADDRESS` — the governor contract address (required)
- `TIMELOCK_ADDRESS` — the timelock contract address (required)

**Script behavior:**
1. `vm.startBroadcast(timelockAddress)` — the timelock is the broadcast address for governor+timelock senders (matches `ResolvedSender::Governor::broadcast_address()`)
2. Deploy `Counter` via `new`
3. Emit `ContractDeployed` with deployer=timelock, standard `DeploymentDetails`
4. Emit `TransactionSimulated` with `senderId = "governance"` and `sender = timelockAddress`

**Acceptance criteria:**
- Script compiles with `forge build`
- `senderId` is `"governance"` (matches the routing classifier in `reduce_queue()`)
- `sender` is the timelock address (matches `broadcast_address()` for Governor+Timelock)
- Script uses the same `DeploymentDetails`, `TxDetails`, `SimTx` struct layout as `Deploy.s.sol` and `DeployViaSafe.s.sol`

---

### P7-US-005: Integration test — Governor → Wallet propose() broadcast on fork

Create `crates/treb-cli/tests/e2e_governor_workflow.rs` with a test that exercises the full governor routing pipeline where the proposer is a wallet.

**Test flow:**
1. Spawn Anvil
2. Copy fixture project to temp dir
3. Deploy governor stack via `deploy_governor()` (short timelock delay, e.g. 1 second)
4. Write `treb.toml` with governor sender config:
   ```toml
   [accounts.proposer_wallet]
   type = "private_key"
   private_key = "<anvil_key_0>"

   [accounts.gov_deployer]
   type = "governance"
   address = "<governor_address>"
   timelock = "<timelock_address>"
   proposer = "proposer_wallet"

   [namespace.default]
   senders = { deployer = "gov_deployer", proposer_wallet = "proposer_wallet" }
   ```
5. Run `treb init`
6. Run `treb fork enter --network anvil-31337 --rpc-url <rpc>`
7. Run `treb run script/DeployViaGovernor.s.sol --broadcast --non-interactive` with `GOVERNOR_ADDRESS` and `TIMELOCK_ADDRESS` env vars
8. Verify registry records:
   - `deployments.json`: 1 deployment (Counter), address non-zero
   - `transactions.json`: at least 1 transaction, sender = timelock address, status = `EXECUTED`
   - `governor-txs.json`: 1 entry, status = `pending`, `governorAddress` matches, `timelockAddress` matches, `transactionIds` links to transaction records, `actions` array non-empty
9. Verify registry consistency via `assert_registry_consistent()`

**Acceptance criteria:**
- Test passes with `cargo test -p treb-cli --test e2e_governor_workflow governor_wallet_propose_on_fork`
- `governor-txs.json` contains a valid `GovernorProposal` record
- Transaction status is `EXECUTED` (fork executed through timelock)
- Deployment address is non-zero (Counter was actually deployed on-chain)

---

### P7-US-006: Integration test — Governor → Safe(1/1) propose() chain on fork

Add a second test to `e2e_governor_workflow.rs` that exercises recursive routing: the governor's proposer is a Safe(1/1), so `propose()` is routed through the Safe execution path.

**Test flow:**
1. Spawn Anvil
2. Copy fixture project
3. Deploy Safe(1/1) via `deploy_safe()` with account #0 as owner
4. Deploy governor stack via `deploy_governor()`
5. Write `treb.toml` with chained config:
   ```toml
   [accounts.signer_wallet]
   type = "private_key"
   private_key = "<anvil_key_0>"

   [accounts.safe_proposer]
   type = "safe"
   safe = "<safe_address>"
   signer = "signer_wallet"

   [accounts.gov_deployer]
   type = "governance"
   address = "<governor_address>"
   timelock = "<timelock_address>"
   proposer = "safe_proposer"

   [namespace.default]
   senders = { deployer = "gov_deployer", safe_proposer = "safe_proposer", signer_wallet = "signer_wallet" }
   ```
6. Run `treb init` → `fork enter` → `treb run script/DeployViaGovernor.s.sol --broadcast --non-interactive`
7. Verify registry records:
   - `governor-txs.json`: 1 proposal, `proposedBy` = account #0 (the Safe signer)
   - `transactions.json`: transaction exists, sender = timelock address, status = `EXECUTED`
   - `deployments.json`: 1 deployment (Counter)
8. Verify registry consistency

**Acceptance criteria:**
- Test passes with `cargo test -p treb-cli --test e2e_governor_workflow governor_safe_propose_on_fork`
- Routing exercised depth-2 path: Governor → Safe(1/1) → Wallet
- Both `governor-txs.json` and `safe-txs.json` may have entries (Safe executes the propose() call, not proposes it, since threshold=1)
- Final deployment exists on-chain

---

### P7-US-007: Integration test — Governor proposal with --skip-fork-execution (queued)

Add a third test to `e2e_governor_workflow.rs` that uses `--skip-fork-execution` to verify the `QueuedExecution::GovernanceProposal` path — the proposal is recorded but NOT executed on fork.

**Test flow:**
1. Spawn Anvil
2. Copy fixture project
3. Deploy governor stack via `deploy_governor()`
4. Write `treb.toml` with governor + wallet proposer config (same as US-005)
5. Run `treb init` → `fork enter` → `treb run script/DeployViaGovernor.s.sol --broadcast --non-interactive --skip-fork-execution`
6. Verify registry records:
   - `governor-txs.json`: 1 proposal, status NOT `executed` (should be `pending` or similar queued state)
   - `transactions.json`: linked transactions have status = `QUEUED`
   - `governor-txs.json` entry has `actions` array with target/value/calldata for the Counter deployment
   - `proposalId` is a non-empty string
7. Verify NO deployment exists in `deployments.json` (skipped execution means Counter was not deployed on-chain) OR verify the deployment exists but transaction is QUEUED (depending on how the pipeline records pre-execution deployments)

**Acceptance criteria:**
- Test passes with `cargo test -p treb-cli --test e2e_governor_workflow governor_skip_fork_execution`
- `governor-txs.json` proposal is in a non-executed state
- Linked transactions in `transactions.json` are `QUEUED`
- The `actions` array contains the expected target/value/calldata

---

## Functional Requirements

**FR-1**: GovernanceToken stub must implement ERC20 `transfer`, `balanceOf`, `totalSupply`, plus governance extensions `delegate`, `getVotes`, and `mint`.

**FR-2**: TrebTimelock stub must implement the exact `scheduleBatch`, `executeBatch`, `getMinDelay`, and `grantRole`/`hasRole` signatures that `fork_routing.rs` calls via its `sol!` macro ABI definitions.

**FR-3**: TrebGovernor stub must implement `propose()` with selector `0x7d5e81e2` (OZ Governor), `state(uint256)` returning uint8 (0–7), and `hashProposal()` for proposal ID computation.

**FR-4**: The `DeployGovernance.s.sol` script must set up correct access control: governor has `PROPOSER_ROLE` on timelock, open executor via `EXECUTOR_ROLE` granted to `address(0)`.

**FR-5**: The `DeployViaGovernor.s.sol` script must use `vm.startBroadcast(timelockAddress)` so that `partition_into_runs()` in `routing.rs` correctly matches the broadcast address.

**FR-6**: The `deploy_governor()` helper must parse broadcast artifacts to extract addresses, matching the pattern established by `deploy_safe()`.

**FR-7**: All integration tests must use `spawn_anvil_or_skip()` and return early (not fail) when Anvil is unavailable.

**FR-8**: Registry assertions must use the correct serialization case: `EXECUTED`/`QUEUED` for `TransactionStatus` (SCREAMING_CASE), `pending`/`executed` for `ProposalStatus` (lowercase).

## Non-Goals

- **Governor voting flow**: The stubs do NOT need to implement a full voting cycle (cast votes, reach quorum, queue, execute through governance). Fork routing skips `propose()` and goes directly to timelock execution via impersonation.
- **ERC20 compliance**: The GovernanceToken does NOT need ERC20 approvals, allowances, or permit. Only `transfer`, `balanceOf`, `delegate`, `getVotes`, `mint`, and `totalSupply` are needed.
- **Governor configuration**: No need for `votingDelay()`, `votingPeriod()`, `quorum()`, or `proposalThreshold()` view functions beyond what `state()` requires internally.
- **Timelock cancellation**: No `cancel()` or `cancelBatch()` support needed.
- **Multi-governor tests**: Phase 8 covers mixed sender tests; this phase focuses on single-governor scenarios.
- **Live network broadcasting**: All tests run on Anvil fork; no live network signing is needed.
- **Re-enabling ignored tests**: Phase 7 creates new tests only; no existing ignored tests are unblocked.

## Technical Considerations

### Dependencies
- **Phase 6 complete**: `deploy_safe()` helper and Safe contract stubs are available for the Governor → Safe(1/1) chain test (US-006).
- **Phase 2 complete**: `--skip-fork-execution` flag is available for the queued proposal test (US-007).

### Solidity stub design
- Follow Phase 6's "minimal but functionally correct" pattern: implement only the methods called by `fork_routing.rs`, with correct storage layouts and access control.
- TrebTimelock must track operation states (Unset/Waiting/Ready/Done) with timestamps, because `executeBatch` must verify the operation is Ready (timestamp has passed).
- TrebGovernor's `propose()` must store proposal data so `state()` can return meaningful values. On a test fork, `state()` will likely return `Pending` or `Succeeded` depending on stub logic.
- The sentinel linked-list pattern used by Safe stubs (e.g., `SENTINEL_OWNERS = address(0x1)`) is NOT needed for governor/timelock — OZ uses `mapping(bytes32 => uint256)` for operation timestamps and `AccessControl` mappings for roles.

### Broadcast address semantics
- Governor+Timelock: `broadcast_address()` returns the **timelock** (not the governor) because the timelock is the on-chain executor (`msg.sender` when proposals execute). The `DeployViaGovernor.s.sol` script must use `vm.startBroadcast(timelockAddress)` accordingly.
- `sender_address()` returns the governor for display/identification purposes.

### CreateCall for CREATE transactions
- The CreateCall helper is already deployed at `0xCCCC...01` via `anvil_setCode` in `fork_routing.rs`. Governor CREATE transactions (like deploying Counter) flow through the same DelegateCall wrapping path that Safe uses. No additional setup is needed.

### Registry format
- `governor-txs.json` is a bare JSON map keyed by proposal ID string
- `GovernorAction` in persistence uses string representations: `target` (checksummed hex), `value` (decimal string), `calldata` (0x-prefixed hex)
- `ProposalStatus` serializes as lowercase: `"pending"`, `"executed"`, etc.
- `TransactionStatus` serializes as SCREAMING_CASE: `"EXECUTED"`, `"QUEUED"`

### Test isolation
- Each test creates its own temp directory and Anvil instance via `spawn_anvil_or_skip()`
- Governor deployment uses `forge script` directly (not `treb run`) to avoid registry entries for infrastructure — same pattern as `deploy_safe()`
- Tests requiring both Safe and Governor infrastructure (US-006) deploy them sequentially on the same Anvil instance
