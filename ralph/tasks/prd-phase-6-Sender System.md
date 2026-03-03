# PRD: Phase 6 - Sender System

## Introduction

Phase 6 implements the sender/wallet abstraction layer that bridges treb's
configuration system (Phase 3) with foundry's `foundry-wallets` crate. This
phase takes the `SenderConfig` definitions already parsed and validated by the
config system and resolves them into live `WalletSigner` instances capable of
signing transactions.

The sender system is a critical integration point: it sits between config
resolution (upstream) and script execution (downstream). Phase 5 established
`ScriptConfig.sender()` as an address-only field and `build_script_config()`
extracts only raw addresses. Phase 6 expands this to produce fully-configured
`MultiWalletOpts` / `WalletSigner` instances that can be wired into
`ScriptArgs.wallets` for in-process forge script execution.

Safe and Governor sender types are stubbed here with the minimum viable
interface; their full implementations are deferred to Phase 17.

## Goals

1. **Resolve all signable sender types to `WalletSigner`**: Given a
   `ResolvedConfig` with validated senders, produce `WalletSigner` instances
   for `PrivateKey`, `Ledger`, and `Trezor` sender types.

2. **Wire signers into forge's execution pipeline**: Extend `ScriptConfig` /
   `build_script_config()` to populate `ScriptArgs.wallets` with a
   `MultiWalletOpts` (or directly inject a `MultiWallet`) so forge script
   execution uses treb-configured signers.

3. **Add `foundry-wallets` dependency to the workspace**: Pin
   `foundry-wallets` at the same tag (`v1.5.1`) as other foundry crates and
   wire it into `treb-forge`.

4. **Provide deterministic in-memory signers for testing**: Implement an
   `InMemory` sender variant that produces `WalletSigner::Local` from
   well-known test private keys (matching anvil's default accounts).

5. **Stub Safe and Governor sender resolution**: Return structured errors or
   marker types for Safe/Governor senders that downstream code (Phase 17) will
   replace with real implementations.

## User Stories

### US-001: Add `foundry-wallets` Dependency and Sender Module Scaffold

**Description**: Add `foundry-wallets` to the workspace dependencies and create
the sender resolution module in `treb-forge` (or a new `sender.rs` module)
with the core `ResolvedSender` type and the public API surface.

**Acceptance Criteria**:
- `foundry-wallets = { git = "...", tag = "v1.5.1" }` added to workspace
  `Cargo.toml` `[workspace.dependencies]`
- `foundry-wallets` added to `treb-forge/Cargo.toml` dependencies
- New `sender.rs` module in `treb-forge` with:
  - `ResolvedSender` enum with variants: `Wallet(WalletSigner)`,
    `Safe { safe_address: Address, signer: Box<ResolvedSender> }`,
    `Governor { governor: Address, proposer: Box<ResolvedSender> }`,
    `InMemory(WalletSigner)`
  - `resolve_sender()` function signature:
    `async fn resolve_sender(name: &str, config: &SenderConfig, all_senders: &HashMap<String, SenderConfig>) -> Result<ResolvedSender>`
  - `resolve_all_senders()` function signature:
    `async fn resolve_all_senders(senders: &HashMap<String, SenderConfig>) -> Result<HashMap<String, ResolvedSender>>`
- Module re-exported from `treb-forge/src/lib.rs`
- `cargo check` passes with no errors

---

### US-002: PrivateKey, Ledger, and Trezor Sender Resolution

**Description**: Implement `resolve_sender()` for the three signable sender
types: `PrivateKey` (from raw hex key), `Ledger` (from optional HD path), and
`Trezor` (from optional HD path). These map directly to
`foundry-wallets::WalletSigner` constructors.

**Acceptance Criteria**:
- **PrivateKey resolution**:
  - Parses hex private key string (with or without `0x` prefix) into `B256`
  - Calls `WalletSigner::from_private_key()` to produce `WalletSigner::Local`
  - Returns `TrebError::Config` with actionable message on invalid key format
  - If `address` field is set, verifies the derived address matches; error if
    mismatch
- **Ledger resolution**:
  - Parses `derivation_path` into `LedgerHDPath` (or uses default
    `LedgerHDPath::LedgerLive(0)`)
  - Calls `WalletSigner::from_ledger_path()` (async)
  - Wraps connection errors with user-friendly message ("Is your Ledger
    connected and unlocked?")
- **Trezor resolution**:
  - Parses `derivation_path` into `TrezorHDPath` (or uses default
    `TrezorHDPath::TrezorLive(0)`)
  - Calls `WalletSigner::from_trezor_path()` (async)
  - Wraps connection errors similarly to Ledger
- Unit tests for PrivateKey resolution (Ledger/Trezor require hardware,
  test error paths only)
- `cargo test` passes for the new tests
- `cargo check` passes

---

### US-003: InMemory Sender and Safe/Governor Stubs

**Description**: Implement the `InMemory` sender for deterministic test
accounts and stub implementations for Safe and Governor sender types that
return structured placeholder values.

**Acceptance Criteria**:
- **InMemory sender**:
  - Provides a function `in_memory_signer(index: u32) -> Result<WalletSigner>`
    that derives the same accounts as Anvil's default HD wallet
    (mnemonic: `"test test test test test test test test test test test junk"`,
    derivation path: `m/44'/60'/0'/0/{index}`)
  - Uses `WalletSigner::from_mnemonic()` internally
  - Convenience function `default_test_signers(count: u32) -> Result<Vec<WalletSigner>>`
    returning `count` signers starting from index 0
  - Unit test verifying index 0 produces address
    `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` (anvil account 0)
- **Safe sender stub**:
  - `resolve_sender()` for `SenderType::Safe` returns
    `ResolvedSender::Safe { safe_address, signer }` where `signer` is
    recursively resolved from the referenced sender
  - The `signer` field's `SenderConfig` is looked up from `all_senders`
  - Returns error if referenced signer cannot be resolved
- **Governor sender stub**:
  - `resolve_sender()` for `SenderType::OZGovernor` returns
    `ResolvedSender::Governor { governor, proposer }` where `proposer` is
    recursively resolved
  - Returns error if referenced proposer cannot be resolved
- `resolve_all_senders()` handles all five sender types
- Unit tests for InMemory signer address derivation
- Unit tests for Safe/Governor stub resolution (using PrivateKey as the
  referenced sub-sender)
- `cargo test` passes

---

### US-004: Wire Sender Resolution into ScriptArgs Construction

**Description**: Extend `ScriptConfig` and `build_script_config()` in
`treb-forge/src/script.rs` to accept resolved senders and populate
`ScriptArgs.wallets` so that forge's execution pipeline has access to the
signing keys.

**Acceptance Criteria**:
- `ScriptConfig` gains a new field for wallet configuration (either
  `MultiWalletOpts` or a mechanism to inject `WalletSigner` instances into
  the `ScriptArgs.wallets` field)
- `build_script_config()` updated to:
  - Call `resolve_sender()` for the "deployer" role (or specified role)
  - For `ResolvedSender::Wallet` / `ResolvedSender::InMemory`: extract the
    private key or signer and set `ScriptArgs.wallets` accordingly
  - For `ResolvedSender::Safe` / `ResolvedSender::Governor`: set only the
    `evm.sender` address (the Safe/Governor contract address) — actual
    signing handled in Phase 17
  - Still sets `evm.sender` to the signer's address for all types
- New `build_script_config_with_senders()` (or extend existing function) that
  takes `&HashMap<String, ResolvedSender>` in addition to `&ResolvedConfig`
- Existing `build_script_config()` behavior preserved (backwards compatible)
  for callers that don't need wallet integration
- Unit test: building `ScriptArgs` from a PrivateKey sender sets both
  `evm.sender` and populates the wallet opts with the private key
- Unit test: building `ScriptArgs` from a Safe sender sets `evm.sender` to the
  safe address
- `cargo check` passes

---

### US-005: Sender Resolution Integration Tests and Error Handling

**Description**: Add comprehensive integration tests for the full sender
resolution pipeline and ensure all error paths produce actionable messages.

**Acceptance Criteria**:
- **Integration test: full pipeline** — Create a `ResolvedConfig` with a
  PrivateKey sender, resolve it, build `ScriptConfig`, and verify the
  resulting `ScriptArgs` has correct sender address and wallet configuration
- **Integration test: multi-sender resolution** — Resolve a config with
  multiple senders (deployer=PrivateKey, admin=Safe referencing deployer) and
  verify both resolve correctly
- **Error test: invalid private key** — Verify `resolve_sender()` returns
  `TrebError::Config` with message containing the sender name and "invalid
  private key"
- **Error test: address mismatch** — Verify that when `SenderConfig.address`
  doesn't match the derived address from the private key, an error is returned
  mentioning both addresses
- **Error test: missing referenced sender** — Verify that resolving a Safe
  sender whose `signer` references a non-existent sender produces an error
  (defense-in-depth; config validation should catch this first)
- **Error test: circular reference** — If sender A references sender B and B
  references A, resolution returns an error instead of stack overflow
- All tests pass with `cargo test`
- `cargo clippy` passes with no warnings on new code

## Functional Requirements

- **FR-1**: The sender module MUST resolve `SenderType::PrivateKey` configs
  into `WalletSigner::Local` instances using `foundry-wallets`.

- **FR-2**: The sender module MUST resolve `SenderType::Ledger` configs into
  `WalletSigner::Ledger` instances, supporting custom derivation paths.

- **FR-3**: The sender module MUST resolve `SenderType::Trezor` configs into
  `WalletSigner::Trezor` instances, supporting custom derivation paths.

- **FR-4**: The sender module MUST provide deterministic in-memory signers
  matching Anvil's default HD wallet accounts.

- **FR-5**: The sender module MUST return `ResolvedSender::Safe` for Safe
  sender types with the underlying signer recursively resolved.

- **FR-6**: The sender module MUST return `ResolvedSender::Governor` for
  OZGovernor sender types with the proposer recursively resolved.

- **FR-7**: `build_script_config()` (or a new variant) MUST populate
  `ScriptArgs.wallets` with signer credentials so forge can sign transactions.

- **FR-8**: All error messages from sender resolution MUST include the sender
  name and an actionable fix suggestion.

- **FR-9**: Sender resolution MUST detect and reject circular references
  (Safe→Governor→Safe etc.) with a clear error.

- **FR-10**: Private key sender resolution MUST validate that the derived
  address matches the configured `address` field when both are provided.

## Non-Goals

- **No Safe Transaction Service integration** — Full Safe multisig signing,
  proposal submission, and confirmation polling are deferred to Phase 17.
  Phase 6 only stubs the `ResolvedSender::Safe` variant.

- **No Governor proposal creation** — Full Governor proposal flow is deferred
  to Phase 17. Phase 6 only stubs `ResolvedSender::Governor`.

- **No keystore file support in treb config** — While `foundry-wallets`
  supports keystores, treb's `SenderConfig` schema does not currently have
  keystore fields. This can be added later if needed.

- **No mnemonic support in treb config** — Similarly, `SenderConfig` has no
  mnemonic field. The InMemory sender uses mnemonics internally for test
  accounts, but user-facing mnemonic config is not part of this phase.

- **No AWS KMS or GCP KMS support in treb config** — `SenderConfig` has no
  AWS/GCP fields. These can be added to the config schema in a future phase.

- **No CLI wallet flags** — The `--private-key`, `--ledger`, `--keystore`
  CLI flags (a la forge) are not implemented. Sender selection comes from
  treb config, not CLI flags. CLI overrides via `ResolveOpts.sender_overrides`
  already exist from Phase 3.

- **No transaction broadcasting** — Phase 6 resolves signers and wires them
  into `ScriptArgs`. Actual transaction signing and broadcasting happen during
  script execution in Phase 8/12.

## Technical Considerations

### Dependencies

- **Phase 3 (Configuration System)**: Provides `ResolvedConfig` with validated
  `HashMap<String, SenderConfig>`. Phase 6 assumes config validation has
  already run — required fields are present and cross-references exist.

- **Phase 5 (In-Process Forge)**: Provides `ScriptConfig`, `ScriptArgs`
  construction, and the execution pipeline. Phase 6 extends this to include
  wallet configuration.

- **foundry-wallets v1.5.1**: New dependency. Must be pinned to the same git
  tag as other foundry crates. The `WalletSigner` enum, `WalletOpts`, and
  `MultiWalletOpts` are the primary integration points.

### Key Integration Points

- **`ScriptArgs.wallets: MultiWalletOpts`** — This is how forge's script
  execution receives signing keys. In `ScriptArgs.preprocess()`, it calls
  `self.wallets.get_multi_wallet()` to produce a `MultiWallet` containing
  the signers. Our `ScriptConfig.into_script_args()` must populate this field.

- **`ScriptArgs.evm.sender: Option<Address>`** — Already wired in Phase 5.
  Must continue to be set to the signer's derived address.

- **`Wallets::new(multi_wallet, evm_sender)`** — Called inside
  `ScriptArgs.preprocess()`. The `MultiWallet` must contain our resolved
  signers, and `evm_sender` must match.

### Private Key Handling

Private keys in `SenderConfig.private_key` may be:
- Raw hex: `"0xabcd1234..."` or `"abcd1234..."`
- Environment variable references: `"${MY_PRIVATE_KEY}"` — already expanded
  by the config resolver (Phase 3 loads `.env` and expands `${...}` patterns)

By the time Phase 6 sees the config, env vars are already expanded. The
sender resolver only needs to handle raw hex strings.

### Hardware Wallet Considerations

Ledger and Trezor resolution is async and may fail if the device is not
connected. The resolution function should:
1. Attempt connection
2. On failure, return a clear error suggesting the user check their device
3. Not block or retry — the caller decides whether to retry

### Recursive Resolution for Safe/Governor

Safe senders reference another sender (the `signer`), and Governor senders
reference a `proposer`. These references create a tree that must be resolved
recursively. A visited-set must be maintained to detect cycles.

### Existing Code to Modify

- `crates/treb-forge/Cargo.toml` — add `foundry-wallets` dependency
- `crates/treb-forge/src/lib.rs` — add `pub mod sender;` and re-exports
- `crates/treb-forge/src/script.rs` — extend `ScriptConfig` and
  `build_script_config()` to accept/use resolved senders
- `Cargo.toml` (workspace) — add `foundry-wallets` to
  `[workspace.dependencies]`

### New Files

- `crates/treb-forge/src/sender.rs` — sender resolution module
