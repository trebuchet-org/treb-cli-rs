# PRD: Phase 6 - Deploy Real Safe Contract on Anvil

## Introduction

Phase 6 adds real Gnosis Safe contract infrastructure to the test Anvil fixture so integration tests can exercise the full Safe routing pipeline against actual on-chain contracts. Currently, the routing code in `fork_routing.rs` and `routing.rs` calls `getThreshold()`, `getOwners()`, `nonce()`, `approveHash()`, and `execTransaction()` on Safe addresses — but no tests deploy real Safe contracts on Anvil to verify these paths end-to-end. This phase creates minimal Safe contract stubs, a deployment script, Rust test helpers, and two integration tests covering the Safe(1/1) execution and Safe(n/m) proposal paths on fork.

This builds on Phase 2's `--skip-fork-execution` flag and Phase 1's live signing infrastructure. The contracts and helpers created here are also prerequisites for Phase 7 (Governor stack) and Phase 8 (mixed sender tests).

## Goals

1. **Deploy functional Safe contracts on Anvil** — Minimal Solidity stubs that faithfully implement the Safe interface used by `fork_routing.rs` (threshold, owners, nonce, approveHash, execTransaction) and MultiSend.
2. **Prove Safe(1/1) fork execution works end-to-end** — Integration test that deploys a 1-of-1 Safe, runs a deployment script through it, and verifies `execute_safe_on_fork()` calls approveHash + execTransaction with correct receipts and registry records.
3. **Prove Safe(n/m) proposal path works on fork** — Integration test that deploys a 2-of-3 Safe, runs a deployment script through it, and verifies `QueuedExecution::SafeProposal` is created with correct safe_tx_hash, nonce, and registry records in `safe-txs.json`.
4. **Provide reusable test infrastructure** — Rust helpers and Forge scripts that Phase 7/8 tests can compose with Governor senders without duplicating Safe deployment boilerplate.

## User Stories

### P6-US-001: Add minimal Safe contract stubs to test fixture

**Description:** Create minimal Solidity contracts in `crates/treb-cli/tests/fixtures/project/src/safe/` that implement the exact interface `fork_routing.rs` calls. These are test stubs, not production Safe contracts — they must be functionally correct for the routing code but can omit features the tests don't exercise (modules, fallback handlers, domain separator caching, etc.).

**Contracts to create:**
- `GnosisSafe.sol` — Implements: `setup(address[],uint256,...)`, `getOwners()`, `getThreshold()`, `nonce()`, `approveHash(bytes32)`, `execTransaction(...)`. Must support pre-approved signature verification (v=1 with r=owner address), owner sorting validation, and nonce increment on successful execution.
- `GnosisSafeProxy.sol` — Minimal proxy that delegates all calls to a singleton address stored in slot 0.
- `GnosisSafeProxyFactory.sol` — `createProxyWithNonce(address singleton, bytes initializer, uint256 saltNonce)` that deploys a proxy and calls the initializer.
- `MultiSend.sol` — `multiSend(bytes transactions)` that unpacks and executes batched operations (operation + to + value + dataLength + data). Must support both CALL (0) and DELEGATECALL (1) operations.

**Acceptance criteria:**
- All four contracts compile with `forge build` in the fixture project (solidity 0.8.30)
- `GnosisSafe.execTransaction` validates pre-approved signatures with v=1 encoding (r=left-padded owner address, s=0) matching `build_pre_approved_signatures()` in `fork_routing.rs`
- `GnosisSafe.execTransaction` increments nonce on success
- `GnosisSafe.setup` stores owners (in order) and threshold, rejects zero threshold or threshold > owners.length
- `MultiSend.multiSend` correctly unpacks the packed encoding format used by `treb_safe::encode_multi_send()` (operation[1] + to[20] + value[32] + dataLen[32] + data[N])
- `GnosisSafeProxy` correctly delegates to singleton via DELEGATECALL
- `GnosisSafeProxyFactory.createProxyWithNonce` deploys a proxy, calls initializer, and returns the proxy address
- Typecheck passes: `cargo clippy --workspace --all-targets`

### P6-US-002: Create DeploySafe.s.sol Forge deployment script

**Description:** Create `crates/treb-cli/tests/fixtures/project/script/DeploySafe.s.sol` — a Forge script that deploys the Safe singleton, proxy factory, and MultiSend contracts, then creates a Safe proxy with configurable owners and threshold. The script must be parameterizable via environment variables so test helpers can control the Safe configuration.

**Script behavior:**
1. Deploy `GnosisSafe` singleton
2. Deploy `GnosisSafeProxyFactory`
3. Deploy `MultiSend` (at a deterministic address or return the deployed address for routing config)
4. Call `factory.createProxyWithNonce(singleton, setupCalldata, salt)` where setup encodes the owner list and threshold from env vars
5. Emit treb `ContractDeployed` events for the Safe proxy (so the pipeline records it)
6. Log the deployed addresses via `console.log` or return data for the test helper to parse

**Environment variable interface:**
- `SAFE_OWNERS` — comma-separated owner addresses
- `SAFE_THRESHOLD` — signature threshold (uint)
- `SAFE_SALT_NONCE` — salt for deterministic proxy address (default: 0)

**Acceptance criteria:**
- Script compiles: `forge build` succeeds in fixture project
- Script deploys functional Safe when run with `forge script` against Anvil
- Deployed Safe responds correctly to `getOwners()`, `getThreshold()`, `nonce()` eth_calls
- `execTransaction` works on the deployed proxy (delegates to singleton)
- MultiSend contract is deployed and functional for delegatecall from the Safe
- Typecheck passes: `cargo clippy --workspace --all-targets`

### P6-US-003: Add Rust e2e helper for Safe deployment on Anvil

**Description:** Add a reusable Rust helper function in the e2e test infrastructure that deploys a Safe with configurable owners and threshold on a running Anvil instance. This helper encapsulates running `DeploySafe.s.sol`, parsing the deployed addresses, and returning them for use in subsequent test steps.

**Helper location:** `crates/treb-cli/tests/e2e/mod.rs` (extend existing module) or a new `crates/treb-cli/tests/e2e/safe.rs` submodule.

**Helper interface:**
```rust
pub struct DeployedSafe {
    pub proxy_address: String,      // The Safe proxy contract address
    pub singleton_address: String,  // The GnosisSafe singleton
    pub factory_address: String,    // The proxy factory
    pub multisend_address: String,  // The MultiSend contract
    pub owners: Vec<String>,        // Owner addresses
    pub threshold: u64,             // Signature threshold
}

/// Deploy a Gnosis Safe with the given owners and threshold on the Anvil instance.
pub async fn deploy_safe(
    project_path: &Path,
    rpc_url: &str,
    owners: &[&str],
    threshold: u64,
) -> DeployedSafe;
```

**Acceptance criteria:**
- Helper successfully deploys Safe(1/1) and returns correct addresses
- Helper successfully deploys Safe(2/3) and returns correct addresses
- Deployed Safe addresses can be verified via eth_call (getOwners, getThreshold match inputs)
- Helper is callable from async test context using `tokio::task::spawn_blocking`
- Typecheck passes: `cargo clippy --workspace --all-targets`

### P6-US-004: Create deployment-through-Safe Forge script

**Description:** Create `crates/treb-cli/tests/fixtures/project/script/DeployViaSafe.s.sol` — a Forge script that deploys a contract (e.g., Counter) using a Safe address as the sender. This script is what `treb run` executes to produce `BroadcastableTransactions` with `from = safe_address`, triggering the Safe routing path. It must emit standard treb events (`ContractDeployed`, `TransactionSimulated`) with the Safe as the deployer.

**Script behavior:**
1. Read Safe address from environment (`SAFE_ADDRESS`)
2. `vm.startBroadcast(safeAddress)` — broadcast from the Safe address
3. Deploy a simple contract (Counter or similar)
4. Emit `ContractDeployed` event with deployer = Safe address
5. Emit `TransactionSimulated` with senderId matching the treb.toml sender name
6. `vm.stopBroadcast()`

**Acceptance criteria:**
- Script compiles: `forge build` succeeds in fixture project
- When executed via `treb run`, produces `BroadcastableTransactions` with `from = safe_address`
- The routing layer correctly identifies this as a Safe sender run
- Emitted events use the Safe address as deployer (not the signer's EOA)
- Typecheck passes: `cargo clippy --workspace --all-targets`

### P6-US-005: Integration test — Safe(1/1) wallet broadcast on fork

**Description:** End-to-end integration test that verifies the full Safe(1/1) execution path on fork: deploy Safe → configure sender → run deployment script → verify routing calls `execute_safe_on_fork()` → verify registry records.

**Test flow:**
1. Spawn Anvil, set up project (fixture copy + `treb init`)
2. Deploy Safe(1/1) using `deploy_safe()` helper with Anvil account #0 as sole owner
3. Write `treb.toml` with Safe sender config pointing to deployed proxy address, signer = Anvil account #0
4. Etch MultiSend bytecode at the canonical address (`0x38869bf66a61cF6bDB996A6aE40D5853Fd43B526`) if deploy_safe helper puts it elsewhere, OR update the test to use the actual deployed address
5. Run `treb run script/DeployViaSafe.s.sol --rpc-url <anvil> --broadcast --non-interactive`
6. Assert command succeeds (exit 0)
7. Assert `treb list --json` shows 1 deployment with deployer = Safe address
8. Assert `.treb/deployments.json` has the deployment record
9. Assert `.treb/transactions.json` has transaction with safeContext populated
10. Assert broadcast artifacts exist with valid receipts

**Acceptance criteria:**
- Test passes: `cargo test -p treb-cli --test e2e_safe_workflow -- safe_1of1_broadcast_on_fork`
- Routing path exercises `execute_safe_on_fork()` (approveHash + execTransaction on real contracts)
- Registry records show Safe address as deployer, not the EOA signer
- Transaction records include `safeContext` with correct `safeTxHash` and `safeAddress`
- Test skips cleanly when Anvil is unavailable (using `spawn_anvil_or_skip` pattern)
- Typecheck passes: `cargo clippy --workspace --all-targets`

### P6-US-006: Integration test — Safe(2/3) proposal on fork

**Description:** End-to-end integration test that verifies the Safe(n/m) proposal path on fork: deploy Safe → configure sender → run deployment script → verify `QueuedExecution::SafeProposal` is created → verify `safe-txs.json` registry records.

**Test flow:**
1. Spawn Anvil, set up project (fixture copy + `treb init`)
2. Deploy Safe(2/3) using `deploy_safe()` helper with Anvil accounts #0, #1, #2 as owners, threshold=2
3. Write `treb.toml` with Safe sender config pointing to deployed proxy, signer = Anvil account #0
4. Run `treb run script/DeployViaSafe.s.sol --rpc-url <anvil> --broadcast --non-interactive --skip-fork-execution`
5. Assert command succeeds (exit 0)
6. Assert `.treb/safe-txs.json` exists and contains exactly 1 entry
7. Assert the safe transaction record has:
   - `safeAddress` = deployed Safe proxy address
   - `status` = "QUEUED"
   - `nonce` = 0 (first Safe tx)
   - Non-empty `safeTxHash`
   - `proposedBy` = Anvil account #0 address
   - Non-empty `transactions` array with the inner tx data
   - `transactionIds` linking to recorded deployment transactions
8. Assert `.treb/deployments.json` has the deployment record

**Acceptance criteria:**
- Test passes: `cargo test -p treb-cli --test e2e_safe_workflow -- safe_2of3_proposal_on_fork`
- Routing path queries threshold (=2), takes the proposal branch, creates `QueuedExecution::SafeProposal`
- `--skip-fork-execution` prevents fork simulation of the queued proposal
- Safe transaction hash in registry matches EIP-712 computation from `treb_safe::compute_safe_tx_hash()`
- Test skips cleanly when Anvil is unavailable
- Typecheck passes: `cargo clippy --workspace --all-targets`

## Functional Requirements

- **FR-1:** Minimal Safe contract stubs must implement the exact ABI surface called by `fork_routing.rs`: `getOwners()`, `getThreshold()`, `nonce()`, `approveHash(bytes32)`, `execTransaction(address,uint256,bytes,uint8,uint256,uint256,uint256,address,address,bytes)`, and `setup(address[],uint256,address,bytes,address,address,uint256,address)`.
- **FR-2:** MultiSend stub must implement `multiSend(bytes)` with the packed encoding format `operation[1] + to[20] + value[32] + dataLen[32] + data[N]`, supporting CALL (0) and DELEGATECALL (1) operations.
- **FR-3:** Safe proxy must correctly delegate all calls to the singleton via DELEGATECALL, storing the singleton address in storage slot 0.
- **FR-4:** `execTransaction` must validate pre-approved signatures (v=1, r=owner address padded to 32 bytes, s=0) — this is the signature format `build_pre_approved_signatures()` in `fork_routing.rs` produces.
- **FR-5:** `execTransaction` must verify that at least `threshold` valid owner signatures are provided with owners in ascending address order (Safe contract invariant).
- **FR-6:** `execTransaction` must increment the Safe nonce on successful execution and return `true`.
- **FR-7:** DeploySafe.s.sol must accept owner addresses and threshold via environment variables for test configurability.
- **FR-8:** DeployViaSafe.s.sol must use `vm.startBroadcast(safeAddress)` to produce BroadcastableTransactions that the routing layer identifies as Safe sender runs.
- **FR-9:** Integration tests must use the `spawn_anvil_or_skip()` pattern for clean skipping in restricted environments.
- **FR-10:** Integration tests must not depend on external services (no Safe Transaction Service calls on fork).

## Non-Goals

- **Not deploying production Gnosis Safe contracts** — We use minimal stubs, not the full ~1000-line Safe v1.3.0/v1.4.1 implementation. Stubs implement only the interface the routing code exercises.
- **Not testing Safe Transaction Service integration** — Phase 6 only tests fork-mode paths. Live-mode proposal (POST to Safe TX Service) is out of scope.
- **Not testing hardware wallet signers** — Safe signer in tests uses Anvil's well-known private keys only.
- **Not testing Governor → Safe chaining** — That's Phase 7's scope.
- **Not testing mixed wallet + Safe sender scripts** — That's Phase 8's scope.
- **Not re-enabling existing ignored tests** — Phase 6 adds new tests only; no tests from the ignored inventory are unblocked by this phase.
- **Not deploying the canonical MultiSend at 0x3889...** — Tests may deploy MultiSend at any address and configure accordingly, or use `vm.etch` to place it at the canonical address.

## Technical Considerations

### Dependencies
- **Phase 2** (`--skip-fork-execution`): Required for P6-US-006 (Safe 2/3 proposal test) — without this flag, the CLI would attempt fork simulation of the queued proposal, which requires interactive confirmation or auto-simulation.
- **Phase 1** (live signing): Required for the broadcast pipeline to work on Anvil (signing + `send_raw_transaction`).

### Solidity Compilation
- The Safe stubs must compile with `solidity =0.8.30` (matching the fixture project's pragma).
- Stubs should be minimal to keep compile times fast in the test suite.
- The fixture project's `foundry.toml` already has `optimizer_runs = 0` and `bytecode_hash = "none"`, which is fine for test contracts.

### MultiSend Address
- `treb_safe::multi_send::MULTI_SEND_ADDRESS` is hardcoded to `0x38869bf66a61cF6bDB996A6aE40D5853Fd43B526`. For fork tests, we need MultiSend at this address.
- Options: (a) deploy MultiSend normally and `vm.etch` its bytecode to the canonical address, (b) deploy with CREATE2 to get the canonical address, or (c) deploy anywhere and etch.
- The DeploySafe.s.sol script should handle placing MultiSend at the expected address.

### Pre-Approved Signature Format
- `fork_routing.rs::build_pre_approved_signatures()` produces signatures where each sig is 65 bytes: `r = owner_address` left-padded to 32 bytes, `s = 0x0` (32 bytes), `v = 1` (1 byte).
- The Safe stub's `execTransaction` must validate this exact format — `v=1` means the owner pre-approved via `approveHash()`.
- Owners must be sorted ascending in the signature list (Safe contract invariant).

### Sender Config in treb.toml
- Safe sender config format:
  ```toml
  [accounts.safe_sender]
  type = "safe"
  safe = "0x<deployed_proxy_address>"
  signer = "deployer"
  ```
- The `signer` references another account entry (private key sender).
- Tests must write this dynamically since the proxy address is only known after deployment.

### Test File Organization
- New test file: `crates/treb-cli/tests/e2e_safe_workflow.rs`
- Helper code: extend `crates/treb-cli/tests/e2e/mod.rs` or add `e2e/safe.rs`
- Follows existing e2e patterns: `#[tokio::test(flavor = "multi_thread")]`, `spawn_blocking` for CLI calls, `spawn_anvil_or_skip()` for graceful skip.

### SENDER_CONFIGS Mechanism
- The routing layer identifies Safe runs by matching `BroadcastableTransaction.from` against resolved sender addresses.
- The Forge script must broadcast from the Safe address (`vm.startBroadcast(safeAddress)`) for the routing to detect it as a Safe run.
- The `SENDER_CONFIGS` environment encoding may need the Safe address to be included so the script knows to broadcast from it — verify this during implementation.
