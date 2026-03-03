# PRD: Phase 3 — Configuration System

## Introduction

Phase 3 builds the layered configuration system that every downstream phase
depends on. It introduces the `treb-config` workspace crate, which owns all
treb-specific configuration parsing, merging, and validation. Foundry's own
`foundry.toml` handling is delegated entirely to `foundry-config::Config` — we
build only the treb-specific layers on top.

Three config sources must be supported from day one:

1. **treb.toml** (v2 format with `[accounts.*]` / `[namespace.*]`, plus v1 legacy
   `[ns.*]` read-only)
2. **.treb/config.local.json** (user's stored namespace/network defaults)
3. **.env / .env.local** (environment variable loading via `dotenvy`)

The resolution order is: foundry-config → treb.toml → .env → local config → CLI
flags. This matches the Go version's precedence exactly so users switching from
Go → Rust see identical behavior.

This phase does not implement CLI commands — those belong to Phase 10.

---

## Goals

1. **Config format compatibility** — Parse treb.toml v1 and v2 formats with the
   exact same TOML keys and semantics as the Go version. Read and write
   `.treb/config.local.json` in the same JSON format.

2. **Layered resolution** — Merge configuration from foundry-config, treb.toml,
   .env files, local config, and (stubbed) CLI flags into a single
   `ResolvedConfig` with deterministic precedence.

3. **Sender config parsing** — Parse all five sender types (`private_key`,
   `ledger`, `trezor`, `safe`, `oz_governor`) from both treb.toml and
   foundry.toml's `[profile.*.treb.senders.*]` sections, with environment
   variable expansion.

4. **Actionable validation** — Detect and report config errors with messages that
   tell the user exactly what to fix: missing files, invalid TOML, unknown sender
   types, dangling cross-references.

5. **Test coverage** — Every config source has fixture-based unit tests covering
   happy path, edge cases, and error cases.

---

## User Stories

### US-001: treb-config Crate Scaffold and Config Types

**Description:** Create the `treb-config` workspace crate with its module
structure and define all configuration data types: `SenderType` enum,
`SenderConfig`, `AccountConfig`, `NamespaceRoles`, `ForkConfig`,
`TrebFileConfigV2`, `TrebFileConfigV1`, `LocalConfig`, and `ResolvedConfig`.
These are pure data types with serde derives — no loading logic yet.

**Acceptance Criteria:**
- `crates/treb-config/` directory created with `Cargo.toml` and `src/lib.rs`
- Crate added to `[workspace.members]` in root `Cargo.toml`
- `treb-config` added to `[workspace.dependencies]` with `path = "crates/treb-config"`
- Dependencies: `treb-core`, `serde`, `serde_json`, `toml` (add `toml = "0.8"` to
  workspace deps), `dotenvy` (add `dotenvy = "0.15"` to workspace deps)
- `SenderType` enum: `PrivateKey`, `Ledger`, `Trezor`, `Safe`, `OZGovernor` with
  serde string representations `"private_key"`, `"ledger"`, `"trezor"`, `"safe"`,
  `"oz_governor"`
- `SenderConfig` struct with fields: `type_` (renamed to `type` in TOML),
  `address`, `private_key`, `safe`, `signer`, `derivation_path`, `governor`,
  `timelock`, `proposer` — all optional except `type_`
- `AccountConfig` — same shape as `SenderConfig` (used in treb.toml v2
  `[accounts.*]`)
- `NamespaceRoles` struct: `profile` (Option\<String\>), `senders`
  (HashMap\<String, String\>) — maps role name to account name
- `ForkConfig` struct: `setup` (Option\<String\>)
- `TrebFileConfigV2` struct: `accounts` (HashMap), `namespace` (HashMap),
  `fork` (ForkConfig) — all with `#[serde(default)]`
- `TrebFileConfigV1` struct: `slow` (Option\<bool\>), `ns`
  (HashMap\<String, NamespaceConfigV1\>)
- `NamespaceConfigV1` struct: `profile` (Option\<String\>), `slow`
  (Option\<bool\>), `senders` (HashMap\<String, SenderConfig\>)
- `LocalConfig` struct: `namespace` (String), `network` (String) with
  `Default` impl returning `namespace: "default"`, `network: ""`
- `ResolvedConfig` struct: `namespace` (String), `network` (Option\<String\>),
  `profile` (String), `senders` (HashMap\<String, SenderConfig\>),
  `slow` (bool), `fork_setup` (Option\<String\>), `config_source` (String),
  `project_root` (PathBuf)
- Module structure: `lib.rs` re-exports types from `types.rs` (or submodules)
- `SenderType` derives `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`, implements
  `Display` and `FromStr`
- Unit tests for `SenderType` serde round-trip and Display/FromStr
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-002: Local Config Store (.treb/config.local.json)

**Description:** Implement read/write for `.treb/config.local.json`. This is the
JSON file where `treb config set` persists user defaults (namespace, network).
Must produce identical JSON to the Go version.

**Acceptance Criteria:**
- `local.rs` module in `treb-config` with `load_local_config` and
  `save_local_config` functions
- `load_local_config(project_root: &Path) -> Result<LocalConfig>`:
  - Reads `.treb/config.local.json` relative to project root
  - Returns `LocalConfig::default()` if file does not exist
  - Returns `TrebError::Config` with actionable message on invalid JSON
- `save_local_config(project_root: &Path, config: &LocalConfig) -> Result<()>`:
  - Creates `.treb/` directory if it does not exist
  - Writes JSON with 2-space indentation + trailing newline
  - Output matches Go format: `{"namespace": "default", "network": ""}`
- Round-trip test: save → load → assert equality
- Test: load from nonexistent file returns defaults
- Test: load from invalid JSON returns descriptive error
- Test: save creates `.treb/` directory if missing
- All tests use `tempdir` (add `tempfile = "3"` to workspace dev-deps if needed)
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-003: treb.toml V2 Parser

**Description:** Parse the treb.toml v2 format — the primary treb-specific
config file with `[accounts.*]`, `[namespace.*]`, and `[fork]` sections. Support
environment variable expansion in sender config string fields using `${VAR}`
syntax. Load project-level `treb.toml` from the project root.

**Acceptance Criteria:**
- `trebfile.rs` module in `treb-config` with `load_treb_config_v2(path: &Path) -> Result<TrebFileConfigV2>`
- Parses TOML using the `toml` crate with `TrebFileConfigV2` serde deserialization
- Returns `TrebError::Config` with file path and TOML parse error on invalid input
- Returns `TrebError::Config` if file does not exist (distinct message from parse error)
- `detect_treb_config_format(project_root: &Path) -> TrebConfigFormat`:
  - Returns `V2` if treb.toml contains `[accounts]`, `[namespace]`, or `[fork]`
  - Returns `V1` if treb.toml contains `[ns.` sections
  - Returns `None` if no treb.toml exists
- `TrebConfigFormat` enum: `None`, `V1`, `V2`
- `expand_env_vars(s: &str) -> String` utility: replaces `${VAR_NAME}` with
  environment variable value, leaves unset vars as empty string
- Environment variable expansion applied to all string fields in
  `AccountConfig` / `SenderConfig` after parsing
- Test fixture: `crates/treb-config/tests/fixtures/treb_v2.toml` with accounts,
  namespaces, and fork config matching the Go version's format
- Test: parse v2 fixture, verify account count, namespace count, sender types
- Test: parse with `${VAR}` references, set env vars in test, verify expansion
- Test: detect format correctly identifies v2 vs v1 vs none
- Test: invalid TOML returns actionable error
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-004: treb.toml V1 Parser (Legacy Read-Only)

**Description:** Parse the legacy treb.toml v1 format with `[ns.*]` sections.
This is read-only for backwards compatibility — the v2 format is canonical. V1
config is internally converted to the same resolved representation as v2.

**Acceptance Criteria:**
- `trebfile_v1.rs` module in `treb-config` with
  `load_treb_config_v1(path: &Path) -> Result<TrebFileConfigV1>`
- Parses TOML using the `toml` crate with `TrebFileConfigV1` serde deserialization
- Environment variable expansion applied to all sender config string fields
- `convert_v1_to_resolved(v1: &TrebFileConfigV1, namespace: &str) -> ResolvedSenders`:
  - Merges `ns.default` senders with `ns.<namespace>` senders (namespace overrides
    default)
  - Extracts profile from `ns.<namespace>.profile`
  - Extracts slow flag (namespace-level overrides top-level)
- `ResolvedSenders` struct (or similar): `profile` (String), `senders`
  (HashMap\<String, SenderConfig\>), `slow` (bool)
- Test fixture: `crates/treb-config/tests/fixtures/treb_v1.toml` with `[ns.default]`
  and `[ns.live]` sections matching Go v1 format
- Test: parse v1 fixture, verify namespace config, sender types
- Test: merge default + named namespace, verify override semantics
- Test: slow flag resolution (namespace overrides top-level)
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-005: .env Loading and Foundry Config Integration

**Description:** Load `.env` and `.env.local` files via `dotenvy` with correct
override semantics (`.env.local` overrides `.env`). Integrate with
`foundry-config::Config::load()` for all `foundry.toml` parsing. Parse
treb-specific sender config from `[profile.*.treb.senders.*]` in foundry.toml
as a fallback when no treb.toml exists.

**Acceptance Criteria:**
- `env.rs` module in `treb-config` with `load_dotenv(project_root: &Path)`:
  - Loads `.env` then `.env.local` from project root (`.env.local` overrides)
  - Uses `dotenvy::from_path_override()` or equivalent
  - Silently skips missing files (not an error)
  - Must be called before any config parsing that uses env var expansion
- `foundry.rs` module in `treb-config` with:
  - `load_foundry_config(project_root: &Path) -> Result<foundry_config::Config>`:
    wraps `foundry_config::Config::load()`, maps errors to `TrebError::Config`
  - `extract_treb_senders_from_foundry(config: &foundry_config::Config, profile: &str) -> HashMap<String, SenderConfig>`:
    Parses `[profile.<name>.treb.senders.*]` from foundry.toml's extra fields
    (via `config.extras` or TOML re-parse of foundry.toml)
- `rpc_endpoints(config: &foundry_config::Config) -> HashMap<String, String>`:
  extracts RPC endpoint map from foundry config
- Test: load `.env` file, verify env vars are set
- Test: `.env.local` overrides `.env` values
- Test: missing `.env` files do not error
- Test: `load_foundry_config` succeeds with a valid foundry.toml fixture
- Test: extract treb senders from foundry.toml with `[profile.default.treb.senders.*]`
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-006: Namespace Resolution and Layered Config Merger

**Description:** Implement the namespace resolution algorithm (v2 hierarchical
walk) and the layered config merger that combines all sources into a final
`ResolvedConfig`. This is the core logic that downstream phases call to get the
effective configuration.

**Acceptance Criteria:**
- `resolver.rs` module in `treb-config` with:
  - `resolve_namespace_v2(config: &TrebFileConfigV2, namespace: &str) -> Result<ResolvedNamespace>`:
    - Walks dot-separated hierarchy: for `"production.ntt"`, resolves
      `default` → `production` → `production.ntt`
    - At each level: profile overrides previous; senders accumulate and override
    - Maps role names to `AccountConfig` by looking up `accounts` map
    - Returns error if referenced account does not exist
  - `ResolvedNamespace` struct: `profile` (String), `senders`
    (HashMap\<String, SenderConfig\>), `slow` (bool), `fork_setup` (Option\<String\>)
  - `resolve_config(opts: ResolveOpts) -> Result<ResolvedConfig>`:
    - Takes `ResolveOpts` with `project_root`, `namespace` (Option), `network`
      (Option), `profile_override` (Option)
    - Calls `load_dotenv` first
    - Loads `LocalConfig` for namespace/network defaults
    - Detects treb.toml format, loads appropriate version
    - Falls back to foundry.toml senders if no treb.toml
    - CLI overrides (namespace, network, profile) take highest precedence
    - Populates `ResolvedConfig.config_source` with `"treb.toml (v2)"`,
      `"treb.toml"`, or `"foundry.toml"`
- Test: v2 hierarchical namespace resolution with 3-level deep namespace
- Test: v2 sender role → account mapping with override at deeper level
- Test: full resolution with treb.toml v2 + local config + env overrides
- Test: fallback to foundry.toml senders when no treb.toml
- Test: CLI overrides take precedence over local config
- Test: missing namespace in v2 config returns error with available namespaces listed
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-007: Config Validation and Error Reporting

**Description:** Add validation for the resolved configuration: sender
cross-reference validation, sender type field requirements, and a unified
validation entry point. All errors must be actionable — they should tell the user
exactly what is wrong and how to fix it.

**Acceptance Criteria:**
- `validation.rs` module in `treb-config` with:
  - `validate_config(config: &ResolvedConfig) -> Result<Vec<ConfigWarning>>`:
    - Returns `Ok(warnings)` on success with optional warnings
    - Returns `Err(TrebError::Config)` on hard errors
  - `validate_sender(name: &str, sender: &SenderConfig) -> Result<Vec<ConfigWarning>>`:
    - `private_key`: must have `private_key` field set
    - `ledger` / `trezor`: warns if no `address` or `derivation_path`
    - `safe`: must have `safe` field; `signer` must reference an existing sender
    - `oz_governor`: must have `governor` field; `proposer` must reference existing sender
  - `ConfigWarning` struct: `message` (String), `location` (String — e.g.,
    `"treb.toml [accounts.safe0]"`)
- Cross-reference validation: safe `signer` and oz_governor `proposer` fields
  must reference sender names that exist in the resolved config
- Error messages include:
  - Which config file the error is in
  - Which section/key is problematic
  - What the user should do to fix it (e.g., `"sender 'safe0' references signer
    'deployer' which is not defined in [accounts]. Add [accounts.deployer] to
    treb.toml"`)
- Test: valid config passes validation with no warnings
- Test: safe sender with missing signer reference returns error
- Test: oz_governor with missing proposer reference returns error
- Test: private_key sender without key set returns error
- Test: ledger sender without address generates warning (not error)
- Test: error messages contain actionable fix instructions
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

## Functional Requirements

- **FR-1:** The `treb-config` crate is the single owner of all configuration
  logic. Other crates depend on `treb-config` for resolved configuration — they
  never parse config files directly.

- **FR-2:** `foundry.toml` parsing is delegated entirely to
  `foundry-config::Config::load()`. We do not write a custom foundry.toml parser.
  Only the treb-specific `[profile.*.treb.*]` extra sections are parsed by us.

- **FR-3:** treb.toml v2 is the canonical format. v1 is read-only for backwards
  compatibility and internally converted to the same resolved representation.

- **FR-4:** Environment variable expansion uses `${VAR_NAME}` syntax. Unset
  variables expand to empty string. Expansion is applied to all string fields in
  sender/account configs after TOML parsing.

- **FR-5:** `.env` files are loaded via `dotenvy` before any config parsing.
  `.env.local` overrides `.env`. Missing files are silently ignored.

- **FR-6:** Configuration precedence (highest to lowest): CLI flags → local
  config (`.treb/config.local.json`) → environment variables → treb.toml →
  foundry.toml defaults.

- **FR-7:** Namespace resolution in v2 walks the dot-separated hierarchy
  (`default` → `production` → `production.ntt`), accumulating sender mappings
  with deeper levels overriding shallower ones.

- **FR-8:** Sender config supports five types: `private_key`, `ledger`, `trezor`,
  `safe`, `oz_governor`. Each type has required and optional fields validated
  after resolution.

- **FR-9:** `.treb/config.local.json` is written with 2-space indented JSON
  matching the Go version's output format. The `.treb/` directory is created if
  it doesn't exist.

- **FR-10:** All config errors use `TrebError::Config` with messages that include
  the source file path and a human-readable description of what went wrong.

---

## Non-Goals

- **No CLI commands** — `treb config show`, `treb config set`, and `treb init`
  belong to Phase 10. This phase implements the library that those commands call.
- **No RPC calls** — Network validation that requires chain ID resolution via RPC
  is deferred to Phase 9. This phase only parses the `network` string.
- **No sender instantiation** — Parsing sender config types is in scope; actually
  creating `WalletSigner` instances from those configs is Phase 6.
- **No script metadata parsing** — `@custom:senders` devdoc parsing belongs to
  Phase 7.
- **No foundry.toml writing** — We only read foundry config, never modify it.
- **No config migration** — Automated v1 → v2 conversion is Phase 19.
- **No interactive prompts** — Config selection UIs belong to Phase 20.

---

## Technical Considerations

### Dependencies to Add

| Crate | Version | Purpose |
|---|---|---|
| `toml` | 0.8.x | TOML parsing for treb.toml |
| `dotenvy` | 0.15.x | .env file loading |
| `tempfile` | 3.x | Temp directories for tests (dev-dependency) |

### Foundry Config Extras

`foundry-config::Config` parses `foundry.toml` but does not know about
`[profile.*.treb.*]` sections. These are preserved in the TOML but not
deserialized into foundry's types. Strategy options:

1. Re-read the TOML file and extract `[profile.*.treb]` sections manually
2. Use foundry's `__extras` or remaining TOML table fields

Evaluate during US-005 which approach works with the pinned foundry version.

### Module Layout

```
crates/treb-config/
├── Cargo.toml
├── src/
│   ├── lib.rs           (pub re-exports)
│   ├── types.rs         (all config data types)
│   ├── local.rs         (LocalConfig read/write)
│   ├── trebfile.rs      (treb.toml v2 parser + format detection)
│   ├── trebfile_v1.rs   (treb.toml v1 parser + v1→resolved conversion)
│   ├── env.rs           (.env loading via dotenvy)
│   ├── foundry.rs       (foundry-config integration, treb sender extraction)
│   ├── resolver.rs      (namespace resolution, layered config merging)
│   └── validation.rs    (config validation, cross-reference checks)
└── tests/
    ├── fixtures/
    │   ├── treb_v2.toml
    │   ├── treb_v1.toml
    │   ├── config_local.json
    │   ├── foundry_with_treb.toml
    │   ├── .env.test
    │   └── .env.local.test
    └── (inline unit tests preferred; integration tests if needed)
```

### Compatibility with Go Version

Key compatibility points to verify:

- `LocalConfig` JSON output is byte-identical to Go's
- treb.toml v1 `[ns.*]` sections parse the same sender types with the same field
  names
- treb.toml v2 `[accounts.*]` / `[namespace.*]` use identical TOML keys
- Environment variable expansion uses the same `${VAR}` syntax (not `$VAR`)
- Namespace resolution walk order matches Go's implementation
- Slow mode defaults to `true` when unset (matching Go)
