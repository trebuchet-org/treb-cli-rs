use regex::Regex;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A normalizer replaces non-deterministic content in CLI output so that
/// golden-file comparisons are stable across environments and runs.
pub trait Normalizer {
    fn normalize(&self, input: &str) -> String;
}

// ---------------------------------------------------------------------------
// Chain
// ---------------------------------------------------------------------------

/// Applies a sequence of normalizers in order.
pub struct NormalizerChain {
    normalizers: Vec<Box<dyn Normalizer>>,
}

impl NormalizerChain {
    pub fn new(normalizers: Vec<Box<dyn Normalizer>>) -> Self {
        Self { normalizers }
    }

    /// Default chain with all built-in normalizers.
    ///
    /// Order matters: color first (strip ANSI), then hash before address
    /// (64-hex must match before 40-hex to avoid partial matches).
    pub fn default_chain() -> Self {
        Self::new(vec![
            Box::new(ColorNormalizer),
            Box::new(SpinnerNormalizer),
            Box::new(ForgeWarningNormalizer),
            Box::new(HashNormalizer),
            Box::new(AddressNormalizer),
            Box::new(TimestampNormalizer),
            Box::new(VersionNormalizer),
            Box::new(GitCommitNormalizer),
            Box::new(RepositoryIdNormalizer),
        ])
    }
}

impl Normalizer for NormalizerChain {
    fn normalize(&self, input: &str) -> String {
        let mut result = input.to_string();
        for n in &self.normalizers {
            result = n.normalize(&result);
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Built-in normalizers
// ---------------------------------------------------------------------------

/// Strips ANSI escape codes (colors, cursor movement, etc.).
pub struct ColorNormalizer;

impl Normalizer for ColorNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
        re.replace_all(input, "").into_owned()
    }
}

/// Replaces 40-hex-char Ethereum addresses (0x-prefixed) with `<ADDRESS>`.
pub struct AddressNormalizer;

impl Normalizer for AddressNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(r"0x[0-9a-fA-F]{40}").unwrap();
        re.replace_all(input, "<ADDRESS>").into_owned()
    }
}

/// Replaces 64-hex-char hashes (0x-prefixed) with `<HASH>`.
/// Must run before AddressNormalizer so 64-hex values are matched first.
pub struct HashNormalizer;

impl Normalizer for HashNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(r"0x[0-9a-fA-F]{64}").unwrap();
        re.replace_all(input, "<HASH>").into_owned()
    }
}

/// Replaces ISO-8601 timestamps and relative time strings.
pub struct TimestampNormalizer;

impl Normalizer for TimestampNormalizer {
    fn normalize(&self, input: &str) -> String {
        // ISO-8601: 2024-01-15T12:30:45Z or 2024-01-15T12:30:45+00:00 or with millis
        let iso = Regex::new(
            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?",
        )
        .unwrap();
        let result = iso.replace_all(input, "<TIMESTAMP>");

        // Date-only: 2024-01-15
        let date_only = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
        let result = date_only.replace_all(&result, "<DATE>");

        // Relative times: "5 minutes ago", "1 hour ago", "2 days ago"
        let relative = Regex::new(r"\d+\s+(second|minute|hour|day|week|month|year)s?\s+ago").unwrap();
        relative.replace_all(&result, "<RELATIVE_TIME>").into_owned()
    }
}

/// Replaces semver version strings (e.g., `1.2.3`, `0.1.0-alpha.1`).
pub struct VersionNormalizer;

impl Normalizer for VersionNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(r"\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?").unwrap();
        re.replace_all(input, "<VERSION>").into_owned()
    }
}

/// Replaces full (40-char) git commit hashes that are NOT 0x-prefixed
/// (0x-prefixed ones are caught by Hash/Address normalizers).
/// Also replaces 7-char short hashes after common git indicators like `@` or `commit `.
pub struct GitCommitNormalizer;

impl Normalizer for GitCommitNormalizer {
    fn normalize(&self, input: &str) -> String {
        // Full 40-char lowercase hex not preceded by 0x (those are Ethereum hashes/addresses).
        // We match standalone sequences that appear at word boundaries.
        let full = Regex::new(r"\b[0-9a-f]{40}\b").unwrap();
        let result = full.replace_all(input, "<GIT_COMMIT>");

        // Short 7-char commit hash after `@` or `commit `
        let short = Regex::new(r"(@|commit )([0-9a-f]{7})\b").unwrap();
        short.replace_all(&result, "${1}<GIT_SHORT>").into_owned()
    }
}

/// Replaces UUID-like repository IDs.
pub struct RepositoryIdNormalizer;

impl Normalizer for RepositoryIdNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(
            r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
        )
        .unwrap();
        re.replace_all(input, "<REPO_ID>").into_owned()
    }
}

/// Strips spinner/progress indicator characters and lines.
pub struct SpinnerNormalizer;

impl Normalizer for SpinnerNormalizer {
    fn normalize(&self, input: &str) -> String {
        // Remove lines that are purely spinner characters (braille dots, common spinner chars)
        let re = Regex::new(r"(?m)^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏⣾⣽⣻⢿⡿⣟⣯⣷|/\-\\]+\s*$\n?").unwrap();
        let result = re.replace_all(input, "");

        // Also remove carriage returns used by spinners to overwrite lines
        let cr = Regex::new(r"\r[^\n]").unwrap();
        cr.replace_all(&result, "").into_owned()
    }
}

/// Strips common Forge warning/info lines that vary between environments.
pub struct ForgeWarningNormalizer;

impl Normalizer for ForgeWarningNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(r"(?m)^(Warning|warning|WARNING):?\s.*$\n?").unwrap();
        re.replace_all(input, "").into_owned()
    }
}

/// Replaces hashes only when preceded by specific prefixes (Tx, Hash, Init Code Hash,
/// Bytecode Hash). Unlike the broad `HashNormalizer`, this avoids matching random
/// 64-hex strings in output and preserves the `0x` prefix in the replacement.
pub struct TargetedHashNormalizer;

impl Normalizer for TargetedHashNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(
            r"(Tx|Hash|Init Code Hash|Bytecode Hash): 0x[a-fA-F0-9]{64}",
        )
        .unwrap();
        re.replace_all(input, "${1}: 0x<HASH>").into_owned()
    }
}

/// Replaces git commit hashes only when preceded by specific labels.
/// `Git Commit: <40-hex>` → `Git Commit: <GIT_COMMIT>` and
/// `commit: <7-hex>` → `commit: <COMMIT>`.
pub struct TargetedGitCommitNormalizer;

impl Normalizer for TargetedGitCommitNormalizer {
    fn normalize(&self, input: &str) -> String {
        let full = Regex::new(r"Git Commit: [a-f0-9]{40}").unwrap();
        let result = full.replace_all(input, "Git Commit: <GIT_COMMIT>");

        let short = Regex::new(r"commit: [a-f0-9]{7}").unwrap();
        short.replace_all(&result, "commit: <COMMIT>").into_owned()
    }
}

/// Removes terminal line-clear escape sequences (`\x1b[2K`) optionally preceded
/// by a carriage return.
pub struct LineClearArtifactNormalizer;

impl Normalizer for LineClearArtifactNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(r"\r?\x1b\[2K").unwrap();
        re.replace_all(input, "").into_owned()
    }
}

/// Removes entire `level=DEBUG` log lines from output.
pub struct DebugNormalizer;

impl Normalizer for DebugNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(r"(?mi)^level=DEBUG.*\n?").unwrap();
        re.replace_all(input, "").into_owned()
    }
}

/// Normalizes forge output that varies between environments:
/// - CRLF → LF, standalone \r removal
/// - Foundry nightly build warning removal
/// - Compiler warning block collapse to `Compiler run successful!\n`
/// - Triple+ blank lines → double blank line
pub struct ForgeOutputNormalizer;

impl Normalizer for ForgeOutputNormalizer {
    fn normalize(&self, input: &str) -> String {
        // Normalize line endings: \r\n → \n, then strip remaining standalone \r
        let result = input.replace("\r\n", "\n");
        let result = result.replace('\r', "");

        // Remove Foundry nightly warning
        let nightly =
            Regex::new(r"(?m)^Warning: This is a nightly build of Foundry\.[^\n]*\n?").unwrap();
        let result = nightly.replace_all(&result, "");

        // Remove "Compiler run successful with warnings:" + all warning lines until a blank line
        let compiler_warnings =
            Regex::new(r"Compiler run successful with warnings:\n(?:[^\n]+\n)*?\n").unwrap();
        let result = compiler_warnings.replace_all(&result, "Compiler run successful!\n");

        // Clean up triple+ blank lines
        let blank_lines = Regex::new(r"\n{3,}").unwrap();
        blank_lines.replace_all(&result, "\n\n").into_owned()
    }
}

/// Replaces all occurrences of a specific label string with `<LABEL>`.
/// Unlike unit-struct normalizers, this accepts a label at construction time.
pub struct LabelNormalizer {
    label: String,
}

impl LabelNormalizer {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

impl Normalizer for LabelNormalizer {
    fn normalize(&self, input: &str) -> String {
        input.replace(&self.label, "<LABEL>")
    }
}

/// Handles bytecode differences in legacy Solidity versions (< 0.8.0).
/// Normalizes `bytecodeHash`, `initCodeHash` JSON fields and `Gas:` values.
pub struct LegacySolidityNormalizer;

impl Normalizer for LegacySolidityNormalizer {
    fn normalize(&self, input: &str) -> String {
        let bytecode =
            Regex::new(r#""bytecodeHash":\s*"0x[a-fA-F0-9]{64}""#).unwrap();
        let result = bytecode.replace_all(input, r#""bytecodeHash": "0x<BYTECODE_HASH>""#);

        let init_code =
            Regex::new(r#""initCodeHash":\s*"0x[a-fA-F0-9]{64}""#).unwrap();
        let result = init_code.replace_all(&result, r#""initCodeHash": "0x<INIT_CODE_HASH>""#);

        let gas = Regex::new(r"Gas:\s*\d+").unwrap();
        gas.replace_all(&result, "Gas: <GAS_AMOUNT>").into_owned()
    }
}

/// Replaces absolute paths with `<PROJECT_ROOT>` for stable golden files.
/// Paths are sorted longest-first to avoid partial matches.
pub struct PathNormalizer {
    paths: Vec<String>,
}

impl PathNormalizer {
    pub fn new(paths: Vec<String>) -> Self {
        let mut paths = paths;
        paths.sort_by(|a, b| b.len().cmp(&a.len())); // longest first
        Self { paths }
    }
}

impl Normalizer for PathNormalizer {
    fn normalize(&self, input: &str) -> String {
        let mut result = input.to_string();
        for path in &self.paths {
            result = result.replace(path, "<PROJECT_ROOT>");
        }
        result
    }
}

/// Replaces epoch-millisecond timestamps (10–13 digit sequences) that appear
/// in backup paths like `prune-1709567890123` or `reset-1709567890123`
/// with the prefix preserved and the number replaced by `<EPOCH>`.
pub struct EpochNormalizer;

impl Normalizer for EpochNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(r"(prune|reset|migrate|backup|bak)-(\d{10,13})\b").unwrap();
        re.replace_all(input, "${1}-<EPOCH>").into_owned()
    }
}

/// Replaces short hex strings (7–10 chars) that appear in version output
/// but aren't caught by `GitCommitNormalizer` (which requires specific
/// prefixes like `@` or `commit `).
pub struct ShortHexNormalizer;

impl Normalizer for ShortHexNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = Regex::new(r"\b[0-9a-f]{7,10}\b").unwrap();
        re.replace_all(input, "<SHORT_HASH>").into_owned()
    }
}

/// Normalizes solc compilation progress lines in forge output.
/// E.g., `Compiling 5 files with solc 0.8.21` → `Compiling <N> files with solc <SOLC_VERSION>`
/// Also handles `Solc 0.8.21 finished in ...` lines.
pub struct CompilerOutputNormalizer;

impl Normalizer for CompilerOutputNormalizer {
    fn normalize(&self, input: &str) -> String {
        // "Compiling N files with solc X.Y.Z" or "Compiling N file with Solc X.Y.Z"
        let compiling = Regex::new(
            r"(?i)Compiling \d+ files? with solc \d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?"
        )
        .unwrap();
        let result = compiling.replace_all(input, "Compiling <N> files with solc <SOLC_VERSION>");

        // "Solc X.Y.Z finished in ..."
        let finished = Regex::new(
            r"(?i)Solc \d+\.\d+\.\d+(-[a-zA-Z0-9.]+)? finished"
        )
        .unwrap();
        finished.replace_all(&result, "Solc <SOLC_VERSION> finished").into_owned()
    }
}

/// Normalizes gas values in forge output.
/// Matches patterns like `gas: 12345`, `Gas used: 54321`, `gasUsed: 123`.
pub struct GasNormalizer;

impl Normalizer for GasNormalizer {
    fn normalize(&self, input: &str) -> String {
        // "gas: NNN", "Gas used: NNN", "gas_used: NNN", "gasUsed: NNN", "gas used: NNN"
        let re = Regex::new(r"(?i)(gas(?:[_ ]?used)?)\s*:\s*\d+").unwrap();
        re.replace_all(input, "${1}: <GAS>").into_owned()
    }
}

/// Normalizes block numbers in forge output.
/// Matches patterns like `block: 1`, `Block: 123`, `blockNumber: 42`, `Block Number: 7`.
pub struct BlockNumberNormalizer;

impl Normalizer for BlockNumberNormalizer {
    fn normalize(&self, input: &str) -> String {
        // "block: N", "Block: N", "blockNumber: N", "Block Number: N"
        let re = Regex::new(r"(?i)(block(?:[_ ]?number)?)\s*:\s*\d+").unwrap();
        re.replace_all(input, "${1}: <BLOCK>").into_owned()
    }
}

/// Normalizes timing/duration strings in forge output.
/// Matches patterns like `1.23s`, `456ms`, `2.5µs`.
pub struct DurationNormalizer;

impl Normalizer for DurationNormalizer {
    fn normalize(&self, input: &str) -> String {
        // Match durations like "1.23s", "456ms", "2µs", "100.5ms"
        let re = Regex::new(r"\d+(?:\.\d+)?(?:µs|ms|s|m)\b").unwrap();
        re.replace_all(input, "<DURATION>").into_owned()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_normalizer_strips_ansi() {
        let n = ColorNormalizer;
        let input = "\x1b[32mSuccess\x1b[0m: deployed \x1b[1;34mContract\x1b[0m";
        assert_eq!(n.normalize(input), "Success: deployed Contract");
    }

    #[test]
    fn address_normalizer_replaces_addresses() {
        let n = AddressNormalizer;
        let input = "deployed to 0x1234567890abcdef1234567890abcdef12345678 on chain 1";
        assert_eq!(
            n.normalize(input),
            "deployed to <ADDRESS> on chain 1"
        );
    }

    #[test]
    fn hash_normalizer_replaces_hashes() {
        let n = HashNormalizer;
        let input =
            "tx 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef done";
        assert_eq!(n.normalize(input), "tx <HASH> done");
    }

    #[test]
    fn hash_before_address_no_false_match() {
        // A 64-hex hash should NOT be partially matched as a 40-hex address
        let chain = NormalizerChain::new(vec![
            Box::new(HashNormalizer),
            Box::new(AddressNormalizer),
        ]);
        let input =
            "tx 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef addr 0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let result = chain.normalize(input);
        assert_eq!(result, "tx <HASH> addr <ADDRESS>");
    }

    #[test]
    fn timestamp_normalizer_handles_iso8601() {
        let n = TimestampNormalizer;

        // Full ISO-8601 with timezone
        assert_eq!(
            n.normalize("at 2024-01-15T12:30:45Z done"),
            "at <TIMESTAMP> done"
        );

        // With offset
        assert_eq!(
            n.normalize("at 2024-01-15T12:30:45+02:00 done"),
            "at <TIMESTAMP> done"
        );

        // With milliseconds
        assert_eq!(
            n.normalize("at 2024-01-15T12:30:45.123Z done"),
            "at <TIMESTAMP> done"
        );
    }

    #[test]
    fn timestamp_normalizer_handles_relative_time() {
        let n = TimestampNormalizer;
        assert_eq!(
            n.normalize("deployed 5 minutes ago"),
            "deployed <RELATIVE_TIME>"
        );
        assert_eq!(
            n.normalize("created 1 hour ago"),
            "created <RELATIVE_TIME>"
        );
    }

    #[test]
    fn version_normalizer() {
        let n = VersionNormalizer;
        assert_eq!(
            n.normalize("treb v0.1.0 (forge 0.2.0)"),
            "treb v<VERSION> (forge <VERSION>)"
        );
    }

    #[test]
    fn normalizer_chain_applies_in_sequence() {
        let chain = NormalizerChain::default_chain();
        let input = "\x1b[32mDeployed\x1b[0m to 0x1234567890abcdef1234567890abcdef12345678 at 2024-01-15T12:30:45Z v0.1.0";
        let result = chain.normalize(input);
        assert_eq!(
            result,
            "Deployed to <ADDRESS> at <TIMESTAMP> v<VERSION>"
        );
    }

    // --- TargetedHashNormalizer ---

    #[test]
    fn targeted_hash_normalizer_replaces_prefixed_hashes() {
        let n = TargetedHashNormalizer;
        let hash = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

        assert_eq!(
            n.normalize(&format!("Tx: {hash} confirmed")),
            "Tx: 0x<HASH> confirmed"
        );
        assert_eq!(
            n.normalize(&format!("Hash: {hash}")),
            "Hash: 0x<HASH>"
        );
        assert_eq!(
            n.normalize(&format!("Init Code Hash: {hash}")),
            "Init Code Hash: 0x<HASH>"
        );
        assert_eq!(
            n.normalize(&format!("Bytecode Hash: {hash}")),
            "Bytecode Hash: 0x<HASH>"
        );
    }

    #[test]
    fn targeted_hash_normalizer_ignores_unprefixed_hashes() {
        let n = TargetedHashNormalizer;
        let hash = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let input = format!("random {hash} value");
        assert_eq!(n.normalize(&input), input);
    }

    // --- TargetedGitCommitNormalizer ---

    #[test]
    fn targeted_git_commit_normalizer_replaces_labeled_commits() {
        let n = TargetedGitCommitNormalizer;

        assert_eq!(
            n.normalize("Git Commit: abcdef1234567890abcdef1234567890abcdef12"),
            "Git Commit: <GIT_COMMIT>"
        );
        assert_eq!(
            n.normalize("commit: abcdef1"),
            "commit: <COMMIT>"
        );
    }

    #[test]
    fn targeted_git_commit_normalizer_ignores_unlabeled() {
        let n = TargetedGitCommitNormalizer;
        let input = "hash abcdef1234567890abcdef1234567890abcdef12 end";
        assert_eq!(n.normalize(input), input);
    }

    // --- LineClearArtifactNormalizer ---

    #[test]
    fn line_clear_artifact_normalizer_removes_escape() {
        let n = LineClearArtifactNormalizer;
        assert_eq!(
            n.normalize("before\x1b[2Kafter"),
            "beforeafter"
        );
        assert_eq!(
            n.normalize("before\r\x1b[2Kafter"),
            "beforeafter"
        );
    }

    #[test]
    fn line_clear_artifact_normalizer_no_match() {
        let n = LineClearArtifactNormalizer;
        let input = "no escape sequences here";
        assert_eq!(n.normalize(input), input);
    }

    // --- DebugNormalizer ---

    #[test]
    fn debug_normalizer_removes_debug_lines() {
        let n = DebugNormalizer;
        let input = "info line\nlevel=DEBUG some debug info\nother line\n";
        assert_eq!(n.normalize(input), "info line\nother line\n");
    }

    #[test]
    fn debug_normalizer_no_match() {
        let n = DebugNormalizer;
        let input = "level=INFO this is fine\nlevel=WARN also fine\n";
        assert_eq!(n.normalize(input), input);
    }

    #[test]
    fn path_normalizer_replaces_paths() {
        let n = PathNormalizer::new(vec!["/tmp/abc123".into()]);
        assert_eq!(
            n.normalize("path /tmp/abc123/foo"),
            "path <PROJECT_ROOT>/foo"
        );
    }

    #[test]
    fn path_normalizer_longest_first() {
        let n = PathNormalizer::new(vec![
            "/tmp/foo".into(),
            "/tmp/foo/bar".into(),
        ]);
        // The longer path should match first, avoiding partial replacement
        assert_eq!(
            n.normalize("path /tmp/foo/bar/baz"),
            "path <PROJECT_ROOT>/baz"
        );
    }

    #[test]
    fn short_hex_normalizer_replaces_short_hashes() {
        let n = ShortHexNormalizer;
        assert_eq!(
            n.normalize("commit abcdef0 done"),
            "commit <SHORT_HASH> done"
        );
        assert_eq!(
            n.normalize("hash abcdef0123 end"),
            "hash <SHORT_HASH> end"
        );
    }

    // --- CompilerOutputNormalizer ---

    #[test]
    fn compiler_output_normalizer_compiling_line() {
        let n = CompilerOutputNormalizer;
        assert_eq!(
            n.normalize("Compiling 5 files with solc 0.8.21"),
            "Compiling <N> files with solc <SOLC_VERSION>"
        );
        // Also handles singular and pre-release
        assert_eq!(
            n.normalize("Compiling 1 file with solc 0.8.26-nightly.2024.1.15"),
            "Compiling <N> files with solc <SOLC_VERSION>"
        );
    }

    #[test]
    fn compiler_output_normalizer_no_match() {
        let n = CompilerOutputNormalizer;
        assert_eq!(
            n.normalize("deployed contract to mainnet"),
            "deployed contract to mainnet"
        );
    }

    #[test]
    fn compiler_output_normalizer_finished_line() {
        let n = CompilerOutputNormalizer;
        assert_eq!(
            n.normalize("Solc 0.8.21 finished in 1.23s"),
            "Solc <SOLC_VERSION> finished in 1.23s"
        );
    }

    // --- GasNormalizer ---

    #[test]
    fn gas_normalizer_replaces_gas_values() {
        let n = GasNormalizer;
        assert_eq!(n.normalize("gas: 12345"), "gas: <GAS>");
        assert_eq!(n.normalize("Gas used: 54321"), "Gas used: <GAS>");
        assert_eq!(n.normalize("gasUsed: 99999"), "gasUsed: <GAS>");
    }

    #[test]
    fn gas_normalizer_no_match() {
        let n = GasNormalizer;
        assert_eq!(
            n.normalize("deployed contract to mainnet"),
            "deployed contract to mainnet"
        );
    }

    // --- BlockNumberNormalizer ---

    #[test]
    fn block_number_normalizer_replaces_block_numbers() {
        let n = BlockNumberNormalizer;
        assert_eq!(n.normalize("block: 1"), "block: <BLOCK>");
        assert_eq!(n.normalize("Block Number: 42"), "Block Number: <BLOCK>");
        assert_eq!(n.normalize("blockNumber: 100"), "blockNumber: <BLOCK>");
    }

    #[test]
    fn block_number_normalizer_no_match() {
        let n = BlockNumberNormalizer;
        assert_eq!(
            n.normalize("deployed contract to mainnet"),
            "deployed contract to mainnet"
        );
    }

    // --- DurationNormalizer ---

    #[test]
    fn duration_normalizer_replaces_durations() {
        let n = DurationNormalizer;
        assert_eq!(n.normalize("finished in 1.23s"), "finished in <DURATION>");
        assert_eq!(n.normalize("took 456ms"), "took <DURATION>");
        assert_eq!(n.normalize("fast: 2µs"), "fast: <DURATION>");
        assert_eq!(n.normalize("total 100.5ms"), "total <DURATION>");
    }

    #[test]
    fn duration_normalizer_no_match() {
        let n = DurationNormalizer;
        assert_eq!(
            n.normalize("deployed contract to mainnet"),
            "deployed contract to mainnet"
        );
    }

    // --- ForgeOutputNormalizer ---

    #[test]
    fn forge_output_normalizer_handles_all_patterns() {
        let n = ForgeOutputNormalizer;

        // CRLF → LF
        assert_eq!(n.normalize("line1\r\nline2\r\n"), "line1\nline2\n");

        // Standalone \r removal
        assert_eq!(n.normalize("before\rafter"), "beforeafter");

        // Foundry nightly warning removal
        let input = "output\nWarning: This is a nightly build of Foundry. Use at your own risk.\nmore output\n";
        assert_eq!(n.normalize(input), "output\nmore output\n");

        // Compiler warning block collapse
        let input = "before\nCompiler run successful with warnings:\nWarning: unused variable\nWarning: shadowed\n\nafter\n";
        assert_eq!(n.normalize(input), "before\nCompiler run successful!\nafter\n");

        // Triple+ blank lines → double blank line
        assert_eq!(n.normalize("a\n\n\n\nb"), "a\n\nb");
    }

    #[test]
    fn forge_output_normalizer_no_match() {
        let n = ForgeOutputNormalizer;
        let input = "clean output\nno warnings\n";
        assert_eq!(n.normalize(input), input);
    }

    // --- LabelNormalizer ---

    #[test]
    fn label_normalizer_replaces_label() {
        let n = LabelNormalizer::new("my-project");
        assert_eq!(
            n.normalize("deployed my-project to mainnet, my-project is live"),
            "deployed <LABEL> to mainnet, <LABEL> is live"
        );
    }

    #[test]
    fn label_normalizer_no_match() {
        let n = LabelNormalizer::new("my-project");
        let input = "no label references here";
        assert_eq!(n.normalize(input), input);
    }

    // --- LegacySolidityNormalizer ---

    #[test]
    fn legacy_solidity_normalizer_replaces_hashes_and_gas() {
        let n = LegacySolidityNormalizer;
        let hash = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

        assert_eq!(
            n.normalize(&format!(r#""bytecodeHash": "{hash}""#)),
            r#""bytecodeHash": "0x<BYTECODE_HASH>""#
        );
        assert_eq!(
            n.normalize(&format!(r#""initCodeHash": "{hash}""#)),
            r#""initCodeHash": "0x<INIT_CODE_HASH>""#
        );
        assert_eq!(
            n.normalize("Gas: 616952"),
            "Gas: <GAS_AMOUNT>"
        );
    }

    #[test]
    fn legacy_solidity_normalizer_no_match() {
        let n = LegacySolidityNormalizer;
        let input = r#""name": "MyContract""#;
        assert_eq!(n.normalize(input), input);
    }
}
