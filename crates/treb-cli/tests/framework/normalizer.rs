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
}
