# Master Plan: treb-cli-rs Drop-In Replacement for treb-cli (Go)

The Rust CLI (`treb-cli-rs`) is being developed as a drop-in replacement for the Go CLI (`treb-cli`). The Rust version has a significant advantage: **in-process foundry integration** via `foundry-rs` crates, eliminating subprocess calls to `forge`. This makes it faster and gives deeper access to compilation, script execution, and anvil node management.

The project already has 7 workspace crates, 22 CLI commands wired up, a golden file test framework with 140+ scenarios, and ~18 phases of work completed covering core types, config, registry, forge integration, sender system, event parsing, deployment pipeline, and basic command implementations. **This master plan covers the remaining work to reach feature parity with the Go CLI and prepare for release.**

Key decisions:
- **Output**: Same commands and flags; `--json` output must be schema-identical; human-readable output should match the Go vibe (tree hierarchy, emoji stages, colored tables) but can improve where possible
- **Forge**: In-process only, no subprocess fallback — this is the key differentiator
- **Registry**: Forward-compatible — Rust reads Go registry files, may add new fields
- **Deprecated features**: Drop treb.toml v1 ongoing support (keep migration path), drop old foundry.toml config format
- **treb-sol**: Keep as git submodule, vendor into a Rust crate for Solidity binding generation
- **Release**: GitHub releases with multi-platform binaries

---

## Phase 1 -- treb-sol Submodule and Solidity Binding Crate

Add the treb-sol Solidity repository as a git submodule and create a dedicated Rust crate that generates type-safe bindings from the Solidity interfaces using `alloy::sol!`. This gives us compile-time-checked event definitions (ITrebEvents), factory interfaces (ICreateX), and deployment script base contracts for template generation. The crate replaces the Go approach of using `abigen` to produce Go bindings.

**Deliverables**
- `.gitmodules` entry for `treb-sol` submodule pointing to `trebuchet-org/treb-sol`
- New `treb-sol` workspace crate with `alloy::sol!` bindings for ITrebEvents, ICreateX, and TrebDeploy interfaces
- Wire `treb-forge` event decoding to use `treb-sol` bindings instead of inline ABI definitions
- Integration test verifying event decoding roundtrip against treb-sol contract ABIs
- Document submodule update workflow in CONTRIBUTING or CLAUDE.md

**User stories:** 5
**Dependencies:** none

**Learnings from implementation:**
- The treb-sol submodule has nested submodules (createx-forge, forge-std, openzeppelin-contracts, safe-utils) — always use `git submodule update --init --recursive`
- The `sol!` macro cannot resolve Solidity `import` statements — inline definitions are required as fallback when source files use imports (e.g., ITrebEvents.sol imports types.sol)
- Each `sol!` block is self-contained; types from one block cannot reference types from another, so related structs and events must be defined in the same block
- treb-sol Rust types intentionally diverge from Solidity source for compatibility with existing treb-forge usage: `string senderId` (not `bytes32`), no `gasUsed` field, `SimulatedTransaction[]` array param on TransactionSimulated event
- Wiring crates to use treb-sol is straightforward: replace `sol! { ... }` blocks with `pub use treb_sol::{Type1, Type2}` re-exports — tests pass unchanged since types are identical
- The treb-sol submodule has no compiled ABI output (no `out/` or `abi/` directories) — JSON ABI comparison requires running `forge build` first
- ICreateX.sol lives in a nested submodule path: `lib/treb-sol/lib/createx-forge/script/ICreateX.sol`

---

## Phase 2 -- Build Metadata and Foundry Version Tracking

Embed foundry version information at compile time so `treb version` reports exactly which foundry commit the binary was built against. This is critical for reproducibility — users need to know their treb binary matches a specific foundry release. Extend `build.rs` to capture foundry crate versions and treb-sol commit hash alongside the existing git commit and build date.

**Deliverables**
- `build.rs` additions: `TREB_FOUNDRY_VERSION` (foundry git tag from Cargo.toml), `TREB_SOL_COMMIT` (treb-sol submodule HEAD)
- `treb version` output includes foundry version, treb-sol commit, rust version, build date
- `treb version --json` schema matches Go (version, commit, date, foundryVersion, trebSolCommit)
- Golden file update for version command output
- Documentation comment in Cargo.toml workspace explaining alloy/foundry version pinning strategy

**User stories:** 4
**Dependencies:** Phase 1

**Notes from Phase 1:**
- treb-sol submodule HEAD can be read via `git -C lib/treb-sol rev-parse HEAD` in build.rs
- The submodule has nested submodules but build.rs only needs the top-level commit hash
- CI workflows must use `submodules: recursive` in checkout action for the submodule to be present at build time

**Learnings from implementation:**
- Foundry version extraction in build.rs uses simple string scanning of workspace Cargo.toml (looks for `foundry-config` line with `tag = "..."`) — no TOML parser build-dependency needed
- `cargo:rerun-if-changed=../../lib/treb-sol` tracks the submodule directory timestamp, which git updates when the submodule pointer changes
- Build env vars set: `TREB_FOUNDRY_VERSION`, `TREB_SOL_COMMIT`, `TREB_GIT_COMMIT`, `TREB_BUILD_DATE`, `TREB_RUST_VERSION` — all accessible via `env!("TREB_*")`
- VersionInfo uses `#[serde(rename_all = "camelCase")]` for Go-compatible JSON; field names (commit, date, foundryVersion, trebSolCommit) are distinct from `Artifact.git_commit` in deployment types
- Golden files auto-regenerate with `UPDATE_GOLDEN=1 cargo test -p treb-cli -- <test_name>` — no manual editing needed
- ShortHexNormalizer handles all 7-10 char hex sequences, so adding new hash fields to output doesn't require normalizer updates
- `output::print_kv` right-aligns labels to the longest one — adding longer labels changes alignment of ALL rows in golden files

---

## Phase 3 -- Output Formatting Framework

Build shared output formatting utilities that match the Go CLI style: tree-style hierarchy rendering, per-element color styling, verification status badges, and responsive terminal width handling. This framework will be consumed by all command output phases that follow. The Go CLI uses `fatih/color` + `go-pretty` tables with yellow namespace backgrounds, cyan chain headers, and tree characters for deployment hierarchy.

**Deliverables**
- Tree renderer: `TreeNode` struct with `render()` producing `|--`, `\--`, `|` prefixed lines
- Color palette module matching Go: namespace (yellow bg), chain (cyan bg), type colors (magenta=proxy, blue=library, green=singleton)
- Verification badge formatter: `e[V] s[-] b[-]` style with color per status
- Fork indicator badge: `[fork]` in yellow
- Unicode-aware column width calculation (handle checkmarks, tree chars, emoji)
- Terminal width detection with graceful fallback for non-TTY (piped output)
- `NO_COLOR` and `TERM=dumb` support in all formatters

**User stories:** 7
**Dependencies:** none

**Notes from Phase 2:**
- `output::print_kv` already exists and right-aligns labels — adding/renaming labels changes alignment of all rows, which cascades to golden file updates
- Golden files auto-regenerate with `UPDATE_GOLDEN=1 cargo test -p treb-cli -- <test_name>` — plan for bulk golden file regeneration when output formatting changes

**Learnings from implementation:**
- UI modules live in `crates/treb-cli/src/ui/` and must be registered in `ui/mod.rs` as `pub mod <name>`
- TreeNode provides `render()` (plain) and `render_styled()` (ANSI) with builder pattern: `TreeNode::new(label).with_style(style).child(child_node)`
- Tree prefixes: `|-- ` (branch), `\-- ` (last), `|   ` (continuation), `    ` (last continuation)
- Color palette in `ui/color.rs` — style constants: NAMESPACE, CHAIN, TYPE_PROXY, TYPE_LIBRARY, TYPE_SINGLETON, TYPE_UNKNOWN, ADDRESS, LABEL, FORK_BADGE, VERIFIED, FAILED, UNVERIFIED (plus existing STAGE, SUCCESS, WARNING, ERROR, MUTED)
- `style_for_deployment_type(DeploymentType) -> Style` mapper in `ui/color.rs` covers all 4 variants
- Badge module (`ui/badge.rs`): `verification_badge()` / `verification_badge_styled()` for `e[V] s[X] b[-]` format; `fork_badge(namespace)` / `fork_badge_styled(namespace)` returns `Some("[fork]")` for `fork/` prefixed namespaces
- Terminal utilities (`ui/terminal.rs`): `terminal_width()` with 80-col fallback for non-TTY; `display_width()` strips ANSI via `console::strip_ansi_codes()` then measures with `unicode_width::UnicodeWidthStr::width()`
- `owo_colors::set_override(true)` must be called per-test to force ANSI output in test environments
- `owo_colors::Style` has no `PartialEq` — compare styles via `format!("{:?}", style)` Debug output in tests
- Cross-module UI integration tests belong in `ui/mod.rs` under `#[cfg(test)] mod ui_integration_tests`
- `display_width()` correctly strips ANSI from styled content, making plain vs styled width comparison a reliable assertion pattern

---

## Phase 4 -- list and show Command Output Parity

Align `treb list` and `treb show` output with Go CLI formatting. The Go `list` command groups deployments by namespace, then chain, then type (Proxies, Implementations, Singletons, Libraries) with tree-style rendering showing proxy-implementation relationships. The Go `show` command displays structured sections (Identity, On-Chain, Transaction, Verification) with per-verifier status.

**Deliverables**
- `list`: namespace -> chain -> deployment type categorical grouping
- `list`: tree hierarchy with proxy -> implementation relationship rows (`\-- <impl_address>`)
- `list`: per-element color styling (namespace bg, chain bg, type-specific text)
- `list`: fork indicator badge on fork-added deployments
- `show`: section headers (Identity, On-Chain, Transaction, Verification, Tags)
- `show`: verification status per verifier with URL display
- `show`: proxy info with implementation address and upgrade history
- Golden file updates for list and show commands

**User stories:** 7
**Dependencies:** Phase 3

**Notes from Phase 3:**
- Use `TreeNode::new(label).with_style(color::NAMESPACE)` for namespace level, `.child()` for chain/deployment nesting — `render_styled()` applies colors to labels only, tree chars remain unstyled
- `style_for_deployment_type()` maps DeploymentType variants to TYPE_PROXY/TYPE_LIBRARY/TYPE_SINGLETON/TYPE_UNKNOWN styles
- `fork_badge(namespace)` returns `Some("[fork]")` for fork namespaces — integrate into list tree labels
- `verification_badge()` produces compact `e[V] s[-] b[-]` string from verifiers HashMap — use in list and show output
- `display_width()` handles ANSI-styled strings correctly for column alignment

**Learnings from implementation:**
- `owo_colors::set_override(false)` does NOT suppress `Style::style()` in owo-colors 4.3.0 — must conditionally use `render()` vs `render_styled()` gated on `color::is_color_enabled()`
- `color::is_color_enabled()` (AtomicBool) is the single source of truth for color state — all styling code must check this, not `set_override`
- `styled()` helper in show.rs: check `is_color_enabled()`, conditionally format with owo_colors — this is the standard pattern for applying color in non-tree contexts
- `print_header()` in show.rs for styled section headers; section separators use `println!(); print_header("X")` for clean styling
- `group_deployments()` uses `BTreeMap` for sorted namespace/chain output with fixed type ordering via `type_sort_key()` (Proxy=0, Singleton=1, Library=2, Unknown=3)
- `build_deployment_node()` creates TreeNode with optional Implementation child for proxy deployments — reuse this pattern for any hierarchical deployment display
- `serde_json` has `preserve_order` feature enabled transitively via alloy crates — `serde_json::Map` uses `IndexMap` not `BTreeMap`, so `to_value()` does NOT sort keys; `sort_json_keys()` in `output.rs` handles recursive key sorting for deterministic `--json` output
- Changing shared fixture `deployments_map.json` cascades to ALL golden file tests using `seed_registry` — must regenerate list AND show golden files together
- `build_from_fixture_deployments` test in `treb-registry/src/lookup.rs` hardcodes deployment/tag counts from the fixture — must update counts when adding deployments or tags
- When adding entries to `deployments_map.json`, omit empty-string fields with `skip_serializing_if = "String::is_empty"` or serde round-trip tests fail
- `VerificationInfo.verifiers` is `HashMap<String, VerifierStatus>` where `VerifierStatus` has `status: String`, `url: String`, `reason: String` — all strings, not enums
- `VERIFIER_DISPLAY_ORDER` constant in show.rs maps lowercase keys to display labels (etherscan/Etherscan, sourcify/Sourcify, blockscout/Blockscout)
- `ProxyInfo` fields: `proxy_type`, `implementation`, `admin`, `history` (Vec<ProxyUpgrade>)
- `print_kv` accepts `&str` — styled values must be pre-computed as `String` and passed by reference

---

## Phase 5 -- run Command Output Parity

Align `treb run` output with Go CLI formatting. The Go command shows staged execution progress with emoji markers, deployment event summaries, gas usage, and transaction details. Debug and verbose modes show additional forge output. Dry-run mode shows what would happen without broadcasting.

**Deliverables**
- Execution progress display with stage indicators (compiling, executing, broadcasting, recording)
- Deployment event summary: contract name, address, label, type per deployment
- Gas usage and transaction hash display
- `--debug` mode: show full forge output, save to debug file
- `--verbose` mode: extra detail on event parsing and registry writes
- `--dry-run` output format: simulated results without broadcast
- `--dump-command` flag: print equivalent forge command (for debugging in-process behavior)
- Golden file updates for run command variants

**User stories:** 7
**Dependencies:** Phase 3

**Notes from Phase 3:**
- Use `TreeNode` for hierarchical deployment event summaries (namespace > chain > deployments)
- Color palette styles (STAGE, SUCCESS, WARNING, ERROR) available for stage indicators
- `terminal_width()` available for responsive output formatting; `display_width()` for column alignment with ANSI content

**Notes from Phase 4:**
- Use `styled()` helper pattern (check `is_color_enabled()`, conditionally format) for all non-tree colored output — do NOT rely on `owo_colors::set_override`
- `build_deployment_node()` in list.rs creates hierarchical TreeNode for a deployment — reuse or follow this pattern for run command deployment summaries
- `print_header()` in show.rs for styled section headers — reuse for execution stage headers
- `print_kv` accepts `&str` — pre-compute styled strings before passing

**Learnings from implementation:**
- `output::print_stage(emoji, message)` writes styled stage progress to stderr; `format_stage()` is the pure-function version for testing — reuse for any multi-step command progress display
- `output::print_warning_banner(emoji, message)` uses `color::WARNING` (yellow) style and prints to stdout — reuse for any warning/notice banners
- `output::eprint_kv(label, value)` is the stderr counterpart of `print_kv` — use for verbose output that must not contaminate stdout/JSON
- `output::format_gas(n)` formats u64 with comma separators (e.g., 1234567 → "1,234,567") — general-purpose, reusable beyond gas
- Stage emojis: `\u{1f528}` compile, `\u{1f4e1}` broadcast, `\u{1f9ea}` simulate, `\u{2705}` complete, `\u{1f6a7}` dry-run banner
- **Separation of concerns**: stage messages go to stderr (progress); banners go to stdout (part of result display)
- `PipelineResult` carries `gas_used: u64` and `event_count: usize` — populated in orchestrator from `ExecutionResult.gas_used` and `parsed_events.len()`
- `RecordedDeployment` wraps `Deployment` — access fields via `rd.deployment.field`; `RecordedTransaction` wraps `Transaction` with `id`, `hash`, `status` fields
- Tree rendering for run output uses same pattern as list: `group_recorded_deployments()` groups by namespace > chain_id > type with BTreeMap; `type_sort_key()` is duplicated from list.rs — consider extracting to shared module
- `ScriptConfig::to_forge_command()` returns `Vec<String>` of equivalent forge CLI args; fields are private so introspection must go through public methods
- Default `sig` is `"run()"` — `to_forge_command()` omits `--sig` when it matches default
- Debug logs saved to `.treb/debug-<timestamp>.log` using `chrono::Utc::now()` formatting
- `DebugLogNormalizer` in test framework normalizes `debug-YYYYMMDD-HHMMSS.log` filenames — add as extra normalizer for any test producing debug logs
- When adding new clap args (e.g., `--dump-command`, `--debug`), must update both the `Commands::Run` enum variant AND the dispatch `match` arm in main.rs
- Verbose pre-execution context must be placed after chain_id resolution (async RPC call) so all values are available
- `--verbose --json` suppression already handled by `if verbose && !json` guards — no special interaction handling needed
- Pre-existing compose golden file failures (JSON key ordering) exist and are not caused by run command changes

---

## Phase 6 -- verify Command Completion

Complete the verification system to support all three block explorers (Etherscan, Blockscout, Sourcify) with proper API interaction, polling, and result display. The Go CLI supports batch verification (`--all`), per-verifier filtering (`--etherscan`, `--blockscout`, `--sourcify`), force re-verification, and watch mode with retries.

**Deliverables**
- Etherscan API: contract submission, status polling, URL extraction
- Blockscout API: contract submission with standard JSON input
- Sourcify API: source file submission
- Batch verification (`--all` flag) with progress indicator
- Per-verifier filtering flags (`--etherscan`, `--blockscout`, `--sourcify`)
- Force re-verification (`--force` flag)
- Watch mode with configurable retries and delay
- Registry update with verification results and URLs
- Output format matching Go: per-verifier status tree with URLs
- Golden file updates

**User stories:** 7
**Dependencies:** Phase 3

**Notes from Phase 3:**
- `verification_badge()` / `verification_badge_styled()` already renders per-verifier status in `e[V] s[X] b[-]` format — use for summary output; for detailed per-verifier tree, use `TreeNode` with VERIFIED/FAILED/UNVERIFIED color styles
- Verifier order is fixed: etherscan, sourcify, blockscout (matching `VERIFIER_ORDER` constant in badge.rs)

**Notes from Phase 4:**
- `VerifierStatus` fields are all `String` (status, url, reason) — not enums; verify command must write these strings to match what show command reads
- `VERIFIER_DISPLAY_ORDER` in show.rs maps lowercase keys (etherscan, sourcify, blockscout) to display labels — verify command must use matching lowercase keys when writing to registry
- Per-verifier detail display is already implemented in show.rs (P4-US-004) — verify command output should be consistent with that format
- `styled()` helper pattern for conditional color — use for verification status coloring (VERIFIED/FAILED/UNVERIFIED styles from color palette)

---

## Phase 7 -- compose Command Completion

Complete the compose orchestration system for multi-step deployment pipelines defined in YAML. The Go CLI supports dependency-ordered execution of multiple scripts, resume from failure, per-component status display, and all the flags from `run` (broadcast, dry-run, verify, debug).

**Deliverables**
- YAML compose file schema validation matching Go format
- Dependency resolution and topological execution ordering
- Per-component status display (pending, running, completed, failed) with tree output
- Resume from failed/interrupted run (`--resume` flag) with state persistence
- All `run` flags forwarded to each component (broadcast, dry-run, verify, slow, legacy)
- Component-level environment variable injection
- Verbose/debug output modes showing per-component details
- Golden file updates

**User stories:** 7
**Dependencies:** Phase 5

**Notes from Phase 5:**
- `print_stage(emoji, message)` for per-component progress indicators (compile, broadcast, simulate, complete) — reuse directly for compose per-component stages
- `print_warning_banner()` for compose-level dry-run banner
- `format_gas()` for per-component and total gas display
- `PipelineResult` carries `gas_used` and `event_count` — aggregate across components for compose summary
- `--dump-command` uses `ScriptConfig::to_forge_command()` — compose could offer per-component command dump
- `--debug` saves to `.treb/debug-<timestamp>.log` — compose may need per-component debug log files or a single combined log
- Stage emojis are established: `\u{1f528}` compile, `\u{1f4e1}` broadcast, `\u{1f9ea}` simulate, `\u{2705}` complete
- `eprint_kv()` for verbose output to stderr — use for per-component verbose context
- Pre-existing compose golden file failures (JSON key ordering) must be fixed before adding new compose golden tests — use `sort_json_keys()` via `print_json` for all JSON output
- `type_sort_key()` is duplicated between list.rs and run.rs — compose should extract to shared module if it also needs deployment type sorting

**Notes from Phase 13:**
- `PipelineResult` now has `governor_proposals: Vec<GovernorProposal>` — compose must aggregate governor proposals across components (same pattern as gas_used/event_count aggregation)
- Compose summary should include governor proposal count when any component uses a governor sender

---

## Phase 8 -- fork Mode Output Parity

Align all fork subcommand outputs with Go CLI formatting. The Go CLI shows fork status tables with uptime, snapshot counts, and deployment counts; history logs with chronological entries; and diff views comparing registry state.

**Deliverables**
- `fork status`: table with network, chain ID, port, uptime, snapshot count, fork-deployment count
- `fork history`: chronological event log with command, snapshot IDs, timestamps
- `fork diff`: deployment diff showing added/modified/removed entries
- `fork enter`: confirmation output with port, PID, initial snapshot ID
- `fork exit`: cleanup confirmation with restored entry count
- Golden file updates for all fork subcommands

**User stories:** 5
**Dependencies:** Phase 3

**Notes from Phase 3:**
- `fork_badge_styled()` renders yellow bold `[fork]` indicator — use in fork status/history output
- `TreeNode` with `render_styled()` can display fork status hierarchy (network > chain > deployments)
- `terminal_width()` for responsive table formatting in fork status output

**Notes from Phase 4:**
- Fork badge is integrated into list command via `fork_badge(namespace)` returning `Some("[fork]")` for `fork/` prefixed namespaces — fork mode output should use the same badge function for consistency
- `styled()` helper pattern for conditional color gated on `is_color_enabled()` — use for all fork subcommand colored output
- `group_deployments()` already handles fork namespaces (sorted alphabetically with non-fork) — fork diff output may want similar grouping

**Notes from Phase 5:**
- `print_stage()` for fork operation stages (entering, snapshotting, reverting, exiting) with appropriate emojis
- `eprint_kv()` for verbose fork context (network, chain ID, RPC, snapshot ID) on stderr
- `print_warning_banner()` for fork-related warnings (e.g., unsaved changes on exit)

---

## Phase 9 -- register and sync Command Completion

Complete `register` for importing existing deployments via transaction tracing and `sync` for polling Safe Transaction Service to update queued transaction status. The Go `register` traces transactions to find all contract creations, offers interactive disambiguation, and handles proxy+implementation pairs.

**Deliverables**
- `register`: `trace_transaction` RPC call to find contract creations within a transaction
- `register`: interactive contract name/path selection (using dialoguer) when multiple matches found
- `register`: proxy + implementation pair detection and linked registration
- `register`: `--skip-verify` flag to skip bytecode verification
- `sync`: Safe Transaction Service API polling for queued transaction status updates
- `sync`: registry update for executed/failed Safe transactions (hash, block number, status)
- `sync`: `--clean` flag to remove invalid/orphaned entries
- Output format matching Go for both commands
- Golden file updates

**User stories:** 7
**Dependencies:** Phase 3, Phase 12 (sync depends on Safe API client, but register can proceed independently)

**Notes from Phase 13:**
- Governor sync is already implemented in sync.rs — Safe sync should follow the same patterns: RPC URL resolution via `resolve_rpc_urls()`, non-fatal warning accumulation, `--clean` flag for stale entry removal
- `SyncOutputJson` already has governor-specific fields (governorSynced, governorUpdated, etc.) — add Safe-specific fields following the same camelCase pattern
- `list_governor_proposals()` borrow-then-clone pattern established for iterating registry entries while mutating — follow for Safe transaction iteration
- `fetch_chain_id` is `pub(crate)` in run.rs and reusable from sync.rs for RPC chain ID resolution

---

## Phase 10 -- tag, prune, and reset Command Parity

Align the three registry management commands with Go output and behavior. These are simpler commands but need proper confirmation prompts, dry-run previews, and formatted output.

**Deliverables**
- `tag`: add/remove tags with confirmation display showing current and updated tag list
- `prune`: on-chain bytecode verification via `eth_getCode` RPC
- `prune`: dry-run preview listing entries to be pruned with reason
- `prune`: interactive confirmation before deletion
- `prune`: `--include-pending` flag for pending Safe/simulated transactions
- `reset`: scope selection by namespace and network
- `reset`: confirmation with item count breakdown and total
- Golden file updates for all three commands

**User stories:** 6
**Dependencies:** Phase 3

---

## Phase 11 -- gen-deploy and migrate Command Parity

Complete script generation from treb-sol templates and config migration from foundry.toml/treb.toml-v1 to treb.toml-v2 format. The Go `gen deploy` generates Solidity scripts inheriting from TrebDeploy with strategy selection. The Go `migrate` reads old config formats, interactively names accounts, and generates the new treb.toml.

**Deliverables**
- `gen-deploy`: template generation using treb-sol base contracts (TrebDeploy)
- `gen-deploy`: CREATE2/CREATE3 strategy selection with factory configuration
- `gen-deploy`: proxy deployment templates (ERC1967, UUPS, Transparent, Beacon) with `--proxy` flag
- `gen-deploy`: `--proxy-contract` flag for custom proxy contract specification
- `migrate`: parse foundry.toml `[profile.*.treb.*]` sections for legacy config
- `migrate`: interactive account naming and namespace pruning prompts
- `migrate`: preview generated treb.toml before writing
- `migrate`: optional foundry.toml cleanup (remove migrated sections)
- Drop treb.toml v1 runtime support (migration path only)
- Golden file updates

**User stories:** 7
**Dependencies:** Phase 1

**Notes from Phase 1:**
- `gen-deploy` templates will reference treb-sol Solidity source files (e.g., TrebDeploy base contract) — these live in `lib/treb-sol/src/` and are available via the submodule
- The `sol!` macro cannot be used for template generation (it produces Rust types, not Solidity source) — `gen-deploy` should read/embed the Solidity source files directly from the submodule path
- treb-sol Rust bindings intentionally diverge from Solidity source in some struct definitions — template-generated Solidity scripts should use the original Solidity types, not the Rust-adapted ones

---

## Phase 12 -- Safe Multisig End-to-End

Complete the Safe multisig integration from proposal through execution tracking. The Go CLI proposes transactions to Safe Transaction Service, tracks confirmation status, and updates the registry when transactions are executed. This requires EIP-712 typed data signing and Safe Transaction Service API interaction.

**Deliverables**
- Safe Transaction Service API client: propose transaction, get status, list pending
- EIP-712 typed data construction and signing for Safe transaction proposals
- `run` command: detect Safe sender type and route to proposal flow instead of direct broadcast
- Safe transaction status tracking in `safe-txs.json` registry
- `sync` command: poll Safe Transaction Service and update tx status (queued -> executed/failed)
- Batch transaction support (multiple operations in single Safe tx)
- Output formatting: Safe proposal confirmation with safeTxHash, required signatures, current confirmations
- Integration tests with mocked Safe Transaction Service responses

**User stories:** 7
**Dependencies:** Phase 5

**Notes from Phase 5:**
- `print_stage()` for Safe proposal flow stages (e.g., proposing, waiting for confirmations, executing)
- `eprint_kv()` for verbose Safe-specific context (safe address, threshold, nonce) on stderr
- `PipelineResult` carries execution stats — Safe proposal flow may need different result fields (safeTxHash, confirmations count) so consider extending or creating a SafeProposalResult
- Run command dispatch in main.rs uses `Commands::Run { ... }` pattern with field destructuring — Safe sender routing should hook into the run command handler after sender resolution
- `--verbose --json` guards already established (`if verbose && !json`) — follow same pattern for Safe-specific verbose output
- Debug log pattern (`.treb/debug-<timestamp>.log`) can capture Safe API request/response details

**Notes from Phase 13:**
- Governor established the pattern for non-EOA sender types: capture `is_governor`/`is_safe` flags BEFORE `deployer_sender` is moved into `PipelineContext`, then use flags for stage message and output routing
- Governor run output added "Creating governance proposal..." stage message — Safe should follow same pattern with "Proposing to Safe..." or similar
- `GovernorProposalJson` struct in run.rs with camelCase serde is the pattern for Safe proposal JSON output
- Run summary includes proposal count with "proposed" / "would be proposed" dry-run language — follow same pattern for Safe proposals
- Governor sync polling in sync.rs is the template for Safe sync: resolve RPC URLs, iterate non-terminal proposals, poll status, update registry, cascade status to linked transactions
- `reqwest` is now available in treb-forge via workspace dep — reuse for Safe Transaction Service API client
- Golden test patterns: `pre_setup_hook` for custom treb.toml sender config, `post_setup_hook` with `Registry::open()` for seeding proposals — reuse for Safe test setup

---

## Phase 13 -- Governor and Timelock Sender Support

Add OpenZeppelin Governor account type support. When the configured sender is an `oz_governor` type, `treb run` creates a governance proposal instead of executing directly. The proposal goes through voting delay, voting period, and timelock before execution.

**Deliverables**
- Governor proposal creation: encode calldata as governance proposal
- Timelock queue and execution flow support
- Proposal status tracking in `governor-txs.json` registry
- `sync` command: poll on-chain governor state for proposal status updates
- Governor account type resolution in sender system
- Output formatting: proposal ID, state, voting period, timelock delay
- Integration tests with mocked governance contracts

**User stories:** 5
**Dependencies:** Phase 5

**Notes from Phase 5:**
- `print_stage()` for governance proposal stages (proposing, voting, queuing, executing)
- `eprint_kv()` for verbose governor context (governor address, proposal ID, voting period) on stderr
- Run command dispatch pattern: sender type determines execution flow — governor routing hooks in same location as Safe routing (after sender resolution in run handler)
- `PipelineResult` may need governor-specific fields (proposalId, state) — follow same pattern as gas_used/event_count additions to types.rs + orchestrator.rs population
- Debug log captures full execution context — extend for governor proposal details

**Learnings from implementation:**
- `TrebError::Governor(String)` variant added to `treb-core/src/error.rs` for governor-specific errors
- `governor_proposals: Vec<GovernorProposal>` added to `PipelineResult` — orchestrator populates in both live and dry-run paths; live path uses `for proposal in &governor_proposals` (borrow) with `.clone()` to keep ownership for the result
- When adding PipelineResult fields: update orchestrator (both paths), `build_pipeline_result()` test helper in pipeline_integration.rs, and all downstream consumers (run.rs, compose)
- `deployer_sender` is moved into `PipelineContext` — capture sender type flags (`is_governor`, `is_safe`) BEFORE the move into PipelineContext
- Governor on-chain polling lives in `crates/treb-forge/src/governor.rs` — `query_proposal_state()`, `map_onchain_state()`, `is_terminal()` re-exported from `treb_forge`
- Governor state selector `0x3e4f49e6` + ABI-encoded uint256 proposal_id for `eth_call`; OZ Governor uint8 states (0-7) mapped to ProposalStatus variants; Expired maps to Defeated
- `list_governor_proposals()` returns `Vec<&GovernorProposal>` (borrowed) — must `.cloned().collect()` before mutating registry in a loop
- Sync governor RPC URL resolution: probe foundry.toml `[rpc_endpoints]` via `eth_chainId` to build chain_id → URL map; `super::run::fetch_chain_id` is `pub(crate)` and reusable from sync.rs
- `err_str.contains("call reverted")` detects contracts that revert on `state()` query (for `--clean` removal of stale proposals)
- Golden test patterns: `pre_setup_hook` to overwrite treb.toml with custom sender config; `post_setup_hook` with `Registry::open()` + `insert_governor_proposal()` to seed proposals before sync
- Governor sender golden tests need BOTH "anvil" (for script senderId) and "deployer" (for governor detection) in treb.toml senders
- Review fix commits can introduce golden file regressions (e.g., `comfy_table` column width changes) — always run full `cargo test -p treb-cli` after any golden file updates
- `reqwest` was already a workspace dep but not declared in treb-forge — just add `reqwest = { workspace = true }` to the crate's Cargo.toml

---

## Phase 14 -- dev anvil Tooling and Improvements

Complete the `dev anvil` subcommands and improve the local development experience. The Go CLI supports start/stop/restart/status/logs for local anvil instances with PID file tracking and log streaming.

**Deliverables**
- `dev anvil restart`: stop + start with same configuration
- `dev anvil logs`: stream anvil log file (tail -f style)
- `dev anvil status`: improved output with PID, port, chain ID, uptime, RPC URL
- PID file and log file management matching Go conventions
- Named instance support (`--name` flag) for multiple concurrent anvil nodes
- Golden file updates

**User stories:** 4
**Dependencies:** none

---

## Phase 15 -- JSON Output Audit and Non-Interactive Mode

Systematic audit of all commands to ensure `--json` output schema matches Go exactly and non-interactive mode works correctly everywhere. This is a cross-cutting concern that touches every command.

**Deliverables**
- Audit and fix `--json` output schema for all 22 commands against Go reference
- Non-interactive mode detection: `TREB_NON_INTERACTIVE=true`, `CI=true`, `NO_COLOR`
- All interactive prompts (dialoguer) bypassed in non-interactive mode with sensible defaults or errors
- JSON error output format: `{"error": "message"}` on stderr
- Exit code parity with Go (0 = success, 1 = error)
- Golden files for JSON output mode of each command

**User stories:** 6
**Dependencies:** Phases 4-14 (all command phases)

**Notes from Phase 2:**
- VersionInfo already uses `#[serde(rename_all = "camelCase")]` for Go-compatible JSON — audit should verify all command output structs follow the same pattern
- ShortHexNormalizer in integration tests handles all 7-10 char hex sequences — no per-field normalizer updates needed when auditing JSON fields containing hashes

**Notes from Phase 4:**
- `serde_json` has `preserve_order` enabled transitively via alloy — `serde_json::Map` uses `IndexMap` not `BTreeMap`, so `to_value()` does NOT produce sorted keys; `sort_json_keys()` in `output.rs` recursively sorts all object keys for deterministic `--json` output — ensure all `--json` code paths use `print_json` (which calls `sort_json_keys`) rather than raw `serde_json::to_string_pretty`
- Changing `deployments_map.json` fixture cascades to ALL golden file tests — JSON audit changes must regenerate golden files for every command that reads from the shared fixture

**Notes from Phase 5:**
- `RunOutputJson` struct exists with `gas_used` field for run command JSON output — verify schema matches Go `treb run --json` exactly
- JSON dry-run output path is completely separate from human output changes — dry-run JSON format was not modified during Phase 5 output parity work
- `--verbose --json` guards use `if verbose && !json` — audit should verify no verbose output leaks into JSON mode for all commands
- `--dump-command` returns early before pipeline execution — no JSON output produced; verify this matches Go behavior

**Notes from Phase 13:**
- `RunOutputJson` now includes `governorProposals` array (camelCase) — audit must verify this matches Go schema; `output::print_json` sorts keys alphabetically so `governorProposals` appears between `gas_used` and `skipped`
- `SyncOutputJson` now includes governor-specific fields (governorSynced, governorUpdated, governorWarnings, governorCleaned) — audit against Go schema
- Governor golden files added for both run and sync JSON output — use as reference during JSON audit

**Learnings from implementation:**
- `ui::interactive::is_non_interactive(cli_flag)` is the centralized non-interactive check — checks `--non-interactive` CLI flag, `TREB_NON_INTERACTIVE=true`, `CI=true`, `!stdin.is_terminal()` in that priority order
- `output::print_json_error(msg)` emits `{"error":"msg"}` to stderr — use for all JSON-mode error output
- `Commands::json_flag()` method extracts `--json` from ANY subcommand (including nested ForkSubcommand, DevSubcommand, AnvilSubcommand, MigrateSubcommand) — must be updated when adding new subcommands
- `main()` delegates to `run(cli)` with error handler checking `Commands::json_flag()` to switch between JSON (`print_json_error`) and text (`{err:?}` Debug) error formats
- Non-JSON errors use `{err:?}` (anyhow Debug) to preserve "Caused by:" chains; JSON errors use `{err:#}` (anyhow alternate Display) for compact single-line chains
- All JSON output structs MUST have `#[serde(rename_all = "camelCase")]` — but structs with Deserialize for file I/O (ComposeFile, ComposeState) should NOT get it if it would break existing file formats
- When adding `--json` to a fork subcommand, update 5 places: (1) enum variant, (2) dispatch match arm, (3) run_* function signature, (4) `ForkSubcommand::json_flag()` in main.rs, (5) clap parsing unit tests
- Guard ALL `print_stage()`, `print_warning_banner()`, `print_kv()`, `eprint_kv()`, `println!()` calls with `if !json` — even stderr output (`print_stage`, `eprint_kv`) should be suppressed in JSON mode for clean output
- `print_warning_banner()` goes to stdout — must ALWAYS be guarded in JSON mode; `print_stage()` goes to stderr — should still be guarded for clean stderr
- `--json --broadcast` without `--non-interactive` should be rejected early with `bail!()` — prevents interactive prompts from blocking JSON-mode consumers (see run.rs and compose.rs patterns)
- `TestWorkdir::new()` always creates `.treb/` dir — use `pre_setup_hook` to remove it for "uninitialized project" error tests
- In subprocess tests, stdin is always piped (non-TTY), so `is_non_interactive()` returns true regardless of env vars — env var tests validate the codepath but the behavior is the same
- `IntegrationTest` builder doesn't support env vars — use `ctx.run_with_env()` directly for env var tests
- Pre-existing golden file mismatches (e.g., table column width differences between environments) should be regenerated as part of any test coverage story to keep CI green
- `output::print_json()` uses `sort_json_keys()` for alphabetical key ordering — golden files must match this sorted order, not struct field declaration order
- selector.rs and prompt.rs use dialoguer for interactive UI; run.rs and compose.rs use manual stdin `read_line` — both must respect `is_non_interactive()` but via different mechanisms

---

## Phase 16 -- End-to-End Workflow Tests

Create comprehensive workflow tests that exercise multi-command sequences matching real user scenarios. These tests validate that commands compose correctly and registry state is consistent across operations.

**Deliverables**
- Workflow: init -> run -> verify -> list -> show (basic deployment)
- Workflow: fork enter -> run -> list -> fork diff -> fork revert -> fork exit
- Workflow: run (Safe sender) -> sync -> show (Safe multisig deployment)
- Workflow: compose -> verify -> list (orchestrated multi-step deployment)
- Workflow: register -> tag -> show (import and annotate existing deployment)
- Workflow: run -> prune -> list (deployment lifecycle with cleanup)
- Workflow: run (Governor sender) -> sync -> show (Governor proposal lifecycle)
- Cross-command registry consistency assertions
- All workflows run against in-process anvil nodes

**User stories:** 6
**Dependencies:** Phases 4-14

**Notes from Phase 13:**
- Governor golden tests demonstrate the test setup patterns: `pre_setup_hook` for treb.toml sender config, `post_setup_hook` for seeding registry data — reuse for E2E workflow test scaffolding
- Governor sync tests produce meaningful output without live RPC ("no RPC endpoint found" warnings) — E2E governor workflow may need a live anvil with a deployed governor contract for full lifecycle testing
- `#[tokio::test(flavor = "multi_thread")]` required for tests using anvil; dry-run tests can use `#[test]`

**Notes from Phase 15:**
- All 23 commands with `--json` now have golden file tests — E2E workflows can rely on JSON output for cross-command assertions (parse with `serde_json::from_str`)
- `is_non_interactive()` returns true in subprocess tests (stdin piped) — E2E workflows won't hang on interactive prompts, but use `TREB_NON_INTERACTIVE=true` or `--non-interactive` explicitly for clarity
- `ctx.run_with_env()` supports setting env vars (e.g., `TREB_NON_INTERACTIVE`, `CI`) in subprocess test contexts
- `--json --broadcast` without `--non-interactive` is rejected early — E2E broadcast workflows must include `--non-interactive` flag when using `--json`
- `TestWorkdir::new()` always creates `.treb/` — use `pre_setup_hook` to customize or remove for specific workflow starting conditions
- JSON error format is `{"error":"msg"}` on stderr with exit code 1 — E2E tests can assert structured error output across command sequences

**Learnings from implementation:**
- E2E test infrastructure: shared helpers in `tests/e2e/mod.rs` imported via `mod e2e;` — includes `setup_project`, `run_deployment`, `spawn_anvil_or_skip`, `treb()`, `run_json`, `assert_deployment_count`, `read_deployments`, `read_transactions`, `assert_registry_consistent`
- Anvil spawning: `AnvilConfig::new().port(0).spawn().await` with `spawn_anvil_or_skip()` wrapper that skips tests when anvil is unavailable
- `treb run --json` stdout contains non-JSON prefix (forge compilation output) — `run_json` helper extracts JSON by finding the first `{` character
- After `treb init`, deployments.json may not exist until first real deployment — check file existence before reading
- Fork snapshot/restore only copies existing files — deploy BEFORE `fork enter` to ensure registry files exist in snapshot; `restore_registry` overwrites from snapshot but does NOT delete files not in snapshot
- Cannot deploy same script twice on same Anvil — causes "transaction already exists" due to deterministic event txId; use tag/reset operations to modify registry during fork instead
- Direct Anvil deploys via `eth_sendTransaction`: poll `eth_getTransactionReceipt` until result is non-null (may be null briefly after send)
- `treb register` args: `--tx-hash`, `--rpc-url`, `--skip-verify` (no `--non-interactive` flag needed — no interactive prompts)
- Contract addresses: compare with `.to_lowercase()` — checksumming may differ between register and receipt
- Creation bytecode `0x6001600c60003960016000f300` correctly deploys 1-byte STOP runtime; `0x6001600a...` has wrong CODECOPY offset → empty runtime bytecode
- Deployment IDs are deterministic: `namespace/chainId/contractName:label` — not unique across redeploys of same contract
- Prune JSON output varies: empty → `{"candidates": []}`, dry-run with candidates → bare array `[...]`, destructive → `{removed: [...], backupPath: "..."}`
- Reset `--namespace` only scopes deployments; transactions/safe-txs/gov-proposals scoped by `--network` only
- `assert_registry_consistent(project_root)` validates bidirectional lookup.json ↔ deployments.json cross-references (byName uses lowercase contract name, byAddress uses lowercase address, byTag uses exact tag string)
- Reset with 0 matching items and `--json` returns Ok(()) with no stdout — `run_json` helper would fail on empty output; ensure queries have > 0 matches when expecting JSON
- New test helpers show `dead_code` warnings until consumed by later stories — expected and harmless

---

## Phase 17 -- Cross-Platform Build and Release Pipeline

Set up GitHub Actions for automated multi-platform binary builds and releases. Match the Go CLI distribution model (GitHub releases with platform-specific binaries) so users can switch from Go to Rust seamlessly.

**Deliverables**
- GitHub Actions workflow: build on push/PR (linux x86_64, macOS x86_64, macOS aarch64)
- GitHub Actions release workflow: triggered on git tag, builds all platforms
- Binary naming convention matching Go: `treb-{os}-{arch}` (e.g., `treb-linux-amd64`)
- Release artifacts: binaries, checksums (SHA256), release notes from git log
- Installation script: `curl -sL ... | bash` for one-liner install
- Foundry version and treb-sol commit included in release notes

**User stories:** 5
**Dependencies:** Phase 2

**Notes from Phase 1:**
- GitHub Actions checkout must use `submodules: recursive` — treb-sol has nested submodules that are required for the build
- CONTRIBUTING.md already documents the submodule workflow (added in Phase 1) — CI config example is included there

**Notes from Phase 2:**
- Build metadata env vars (`TREB_FOUNDRY_VERSION`, `TREB_SOL_COMMIT`) are set in `crates/treb-cli/build.rs` and available at compile time — release notes can extract these from the binary via `treb version --json`
- Foundry version is extracted from workspace Cargo.toml `tag = "vX.Y.Z"` on the `foundry-config` dependency line
- The alloy/foundry version pinning strategy is documented in a comment block in workspace Cargo.toml — CI should validate `cargo check` passes to catch pin breakage

**Notes from Phase 16:**
- E2E tests use `#[tokio::test(flavor = "multi_thread")]` + `tokio::task::spawn_blocking` for async tests with CLI subprocess calls — CI runners must support tokio multi-threaded runtime
- E2E tests spawn live Anvil nodes on OS-assigned ports (`port(0)`) — CI must have foundry/anvil available or tests will be skipped via `spawn_anvil_or_skip()`
- Tests in `tests/e2e_*.rs` files import shared helpers via `mod e2e;` from `tests/e2e/mod.rs` — all E2E test files must be in the `tests/` root directory

**Learnings from implementation:**
- CI workflows live in `.github/workflows/` — ci.yml (4 jobs: check, test, clippy, fmt), release-build.yml (matrix build), release.yml (tag-triggered release), foundry-track.yml
- All checkout steps across ALL workflows need `submodules: recursive` — treb-sol has nested submodules required for build
- `foundry-rs/foundry-toolchain@v1` must come before `rust-cache` in CI for optimal caching
- `cargo test --workspace --all-targets` includes integration tests that `--workspace` alone skips
- Shell completions output path: `target/{target}/release/build/treb-cli-*/out/completions/` — use `find` glob since the hash in the path varies per build
- clap_complete output files: `treb.bash`, `_treb` (zsh), `treb.fish`, `treb.elv` — standard install locations: bash `~/.local/share/bash-completion/completions/`, zsh `~/.local/share/zsh/site-functions/`, fish `~/.config/fish/completions/`
- Release archives use Go-compatible naming: `treb-{tag}-{os_arch}.tar.gz` (linux-amd64, linux-arm64, darwin-amd64, darwin-arm64) — archive contents are flat (treb binary + completions/ dir)
- Binary name is `treb-cli` (Cargo package name), renamed to `treb` only in release packaging stage
- `gh release create` supports both `--notes` and `--generate-notes` together — custom notes appear first, auto-generated changelog appended
- `workflow_run` events need explicit `ref:` in checkout to get the correct tag/branch code
- Build metadata (foundry version, treb-sol commit, Rust version) must be gathered in the release job since it runs in a separate workflow from the build matrix
- GitHub org is `trebuchet-org` — repo is `trebuchet-org/treb-cli-rs`
- `trebSolCommit` in build.rs requires `submodules: recursive` checkout since it runs `git -C ../../lib/treb-sol rev-parse`
- `foundryVersion` is extracted from workspace Cargo.toml `foundry-config` tag field at build time (not runtime forge detection)
- Cross.toml target sections can be empty — cross uses sensible defaults for common musl targets
- CI smoke test validates `treb version --json` metadata: asserts version, commit, foundryVersion, trebSolCommit are non-empty and not "unknown"
- Installation script (`trebup`) supports `--help`, auto-detects shell from `$SHELL` for completions installation, and checks PATH presence with hint to add install dir

---

## Phase 18 -- Documentation and Migration Guide

Write user-facing documentation for the Rust CLI including a migration guide for users switching from the Go version. Highlight the in-process forge advantage, any behavior differences, and dropped features.

**Deliverables**
- README.md: feature overview, installation, quick start, command reference
- MIGRATION.md: Go -> Rust CLI migration guide (breaking changes, dropped features, improvements)
- CHANGELOG.md: structured changelog from git history
- CLAUDE.md updates: architectural decisions, crate responsibilities, testing patterns
- Inline help text (`--help`) audit for all commands to match Go descriptions

**User stories:** 4
**Dependencies:** Phases 1-17

**Notes from Phase 15:**
- Non-interactive mode supports 3 detection methods: `--non-interactive` CLI flag, `TREB_NON_INTERACTIVE=true` env var, `CI=true` env var, plus TTY detection — document all methods in CLI reference
- JSON error format is standardized: `{"error":"msg"}` on stderr, exit code 1 — document for scripting/CI integration guides
- `--json --broadcast` requires `--non-interactive` — document this constraint for CI/CD pipeline examples
- All `--json` output uses alphabetically sorted keys (via `sort_json_keys()`) — document for users parsing JSON output programmatically

**Notes from Phase 16:**
- E2E test patterns established: shared helpers in `tests/e2e/mod.rs`, reusable `assert_registry_consistent()` for bidirectional lookup validation — document in CLAUDE.md testing patterns section
- Deployment ID format is `namespace/chainId/contractName:label` (deterministic, not unique across redeploys) — document for users managing registry state
- Registry behavior nuances worth documenting: `treb init` does not create deployments.json (created on first deploy); fork snapshot only copies existing files; reset `--namespace` scopes deployments only (not transactions)
- `treb run --json` stdout may contain forge compilation output before JSON — document for users parsing JSON output programmatically (search for first `{`)

**Notes from Phase 17:**
- Installation script (`trebup`) supports `--help`, shell completions auto-installation (bash, zsh, fish from `$SHELL` detection), and PATH presence check — document installation in README quick start
- Shell completion install locations: bash `~/.local/share/bash-completion/completions/`, zsh `~/.local/share/zsh/site-functions/`, fish `~/.config/fish/completions/` — document for users who want manual completion setup
- `treb version --json` emits build metadata (version, commit, date, foundryVersion, trebSolCommit, rustVersion) — document for CI integration and version pinning
- GitHub org is `trebuchet-org` — use correct org in all documentation URLs and installation instructions
- Binary name is `treb-cli` (Cargo package), renamed to `treb` in release packaging — document the distinction for users building from source vs installing from release
- CI smoke test validates version JSON metadata fields are non-empty and not "unknown" — reference as testing pattern in CLAUDE.md

---

## Dependency Graph (ASCII)

```
Phase 1 (treb-sol)───────────┬──> Phase 2 (build metadata) ──> Phase 17 (release)
                             │
                             ├──> Phase 11 (gen-deploy/migrate)
                             │
Phase 3 (output framework)───┼──> Phase 4 (list/show)
                             │
                             ├──> Phase 5 (run) ──┬──> Phase 7 (compose)
                             │                    ├──> Phase 12 (safe)
                             │                    └──> Phase 13 (governor)
                             │
                             ├──> Phase 6 (verify)
                             │
                             ├──> Phase 8 (fork)
                             │
                             ├──> Phase 9 (register/sync) ── [soft dep on Phase 12]
                             │
                             └──> Phase 10 (tag/prune/reset)

Phase 14 (dev anvil) ── independent

Phases 4-14 ──> Phase 15 (JSON audit)
Phases 4-14 ──> Phase 16 (E2E tests)
Phases 1-17 ──> Phase 18 (documentation)
```

---

## Summary Table

| Phase | Title | Stories | Depends On |
|------:|-------|--------:|------------|
| 1 | treb-sol Submodule and Solidity Binding Crate | 5 | -- |
| 2 | Build Metadata and Foundry Version Tracking | 4 | 1 |
| 3 | Output Formatting Framework | 7 | -- |
| 4 | list and show Command Output Parity | 7 | 3 |
| 5 | run Command Output Parity | 7 | 3 |
| 6 | verify Command Completion | 7 | 3 |
| 7 | compose Command Completion | 7 | 5 |
| 8 | fork Mode Output Parity | 5 | 3 |
| 9 | register and sync Command Completion | 7 | 3, 12 (soft) |
| 10 | tag, prune, and reset Command Parity | 6 | 3 |
| 11 | gen-deploy and migrate Command Parity | 7 | 1 |
| 12 | Safe Multisig End-to-End | 7 | 5 |
| 13 | Governor and Timelock Sender Support | 5 | 5 |
| 14 | dev anvil Tooling and Improvements | 4 | -- |
| 15 | JSON Output Audit and Non-Interactive Mode | 6 | 4-14 |
| 16 | End-to-End Workflow Tests | 6 | 4-14 |
| 17 | Cross-Platform Build and Release Pipeline | 5 | 2 |
| 18 | Documentation and Migration Guide | 4 | 1-17 |
| **Total** | | **106** | |
