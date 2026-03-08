//! Color output control and palette constants.
//!
//! Respects the `NO_COLOR` environment variable (<https://no-color.org/>) and
//! the `TERM=dumb` convention. Use [`should_use_color`] to check whether color
//! output is appropriate, and [`color_enabled`] to query or apply the decision
//! to the owo-colors subsystem.
//!
//! The palette matches the Go CLI `fatih/color` definitions from
//! `render/deployments.go:19-34` and `render/script.go:17-23`.

use std::sync::atomic::{AtomicBool, Ordering};

use owo_colors::Style;
use treb_core::types::DeploymentType;

/// Resolved color state (set once by [`color_enabled`], queried via [`is_color_enabled`]).
static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// Palette constants – general
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Color/style for stage/section headings.
pub const STAGE: Style = Style::new().cyan().bold();

#[allow(dead_code)]
/// Color/style for success messages.
pub const SUCCESS: Style = Style::new().green().bold();

#[allow(dead_code)]
/// Color/style for warnings.
pub const WARNING: Style = Style::new().yellow();

#[allow(dead_code)]
/// Color/style for errors.
pub const ERROR: Style = Style::new().red().bold();

#[allow(dead_code)]
/// Color/style for muted/secondary text.
pub const MUTED: Style = Style::new().dimmed();

// ---------------------------------------------------------------------------
// Palette constants – deployment-specific (Go: render/deployments.go:19-34)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Namespace header: black text on yellow background (Go: `nsHeader`).
pub const NS_HEADER: Style = Style::new().black().on_yellow();

#[allow(dead_code)]
/// Namespace header bold: black bold on yellow background (Go: `nsHeaderBold`).
pub const NS_HEADER_BOLD: Style = Style::new().black().bold().on_yellow();

#[allow(dead_code)]
/// Chain header: black text on cyan background (Go: `chainHeader`).
pub const CHAIN_HEADER: Style = Style::new().black().on_cyan();

#[allow(dead_code)]
/// Chain header bold: black bold on cyan background (Go: `chainHeaderBold`).
pub const CHAIN_HEADER_BOLD: Style = Style::new().black().bold().on_cyan();

#[allow(dead_code)]
/// Color/style for namespace labels.
pub const NAMESPACE: Style = Style::new().cyan().bold();

#[allow(dead_code)]
/// Color/style for chain labels.
pub const CHAIN: Style = Style::new().magenta().bold();

#[allow(dead_code)]
/// Color/style for proxy deployment type (Go: `FgMagenta, Bold`).
pub const TYPE_PROXY: Style = Style::new().magenta().bold();

#[allow(dead_code)]
/// Color/style for library deployment type (Go: `FgBlue, Bold`).
pub const TYPE_LIBRARY: Style = Style::new().blue().bold();

#[allow(dead_code)]
/// Color/style for singleton deployment type (Go: `FgGreen, Bold`).
pub const TYPE_SINGLETON: Style = Style::new().green().bold();

#[allow(dead_code)]
/// Color/style for unknown deployment type.
pub const TYPE_UNKNOWN: Style = Style::new().dimmed();

#[allow(dead_code)]
/// Color/style for address display (Go: `FgWhite`).
pub const ADDRESS: Style = Style::new().white();

#[allow(dead_code)]
/// Color/style for deployment labels/names.
pub const LABEL: Style = Style::new().bold();

#[allow(dead_code)]
/// Color/style for timestamp display (Go: `Faint`).
pub const TIMESTAMP: Style = Style::new().dimmed();

#[allow(dead_code)]
/// Color/style for pending status (Go: `FgYellow`).
pub const PENDING: Style = Style::new().yellow();

#[allow(dead_code)]
/// Color/style for tags display (Go: `FgCyan`).
pub const TAGS: Style = Style::new().cyan();

#[allow(dead_code)]
/// Color/style for section headers (Go: `Bold, FgHiWhite`).
pub const SECTION_HEADER: Style = Style::new().bold().bright_white();

#[allow(dead_code)]
/// Color/style for implementation prefix (Go: `Faint`).
pub const IMPL_PREFIX: Style = Style::new().dimmed();

#[allow(dead_code)]
/// Color/style for fork indicator (Go: `FgYellow`).
pub const FORK_INDICATOR: Style = Style::new().yellow();

#[allow(dead_code)]
/// Color/style for fork badge indicators (alias for `FORK_INDICATOR`).
pub const FORK_BADGE: Style = Style::new().yellow();

#[allow(dead_code)]
/// Color/style for verified status (Go: `FgGreen`).
pub const VERIFIED: Style = Style::new().green();

#[allow(dead_code)]
/// Color/style for not-verified / failed verification status (Go: `FgRed`).
pub const NOT_VERIFIED: Style = Style::new().red();

#[allow(dead_code)]
/// Color/style for failed verification status (alias for `NOT_VERIFIED`).
pub const FAILED: Style = Style::new().red();

#[allow(dead_code)]
/// Color/style for unverified/missing status.
pub const UNVERIFIED: Style = Style::new().dimmed();

// ---------------------------------------------------------------------------
// Palette constants – script renderer (Go: render/script.go:17-23)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Bold style for script renderer.
pub const BOLD: Style = Style::new().bold();

#[allow(dead_code)]
/// Gray (bright black / FgHiBlack) for script renderer.
pub const GRAY: Style = Style::new().bright_black();

#[allow(dead_code)]
/// Cyan foreground for script renderer.
pub const CYAN: Style = Style::new().cyan();

#[allow(dead_code)]
/// Yellow foreground for script renderer.
pub const YELLOW: Style = Style::new().yellow();

#[allow(dead_code)]
/// Green foreground for script renderer.
pub const GREEN: Style = Style::new().green();

#[allow(dead_code)]
/// Red foreground for script renderer.
pub const RED: Style = Style::new().red();

// ---------------------------------------------------------------------------
// Deployment type → style mapping
// ---------------------------------------------------------------------------

/// Returns the appropriate [`Style`] for a given [`DeploymentType`].
#[allow(dead_code)]
pub fn style_for_deployment_type(dt: DeploymentType) -> Style {
    match dt {
        DeploymentType::Proxy => TYPE_PROXY,
        DeploymentType::Library => TYPE_LIBRARY,
        DeploymentType::Singleton => TYPE_SINGLETON,
        DeploymentType::Unknown => TYPE_UNKNOWN,
    }
}

// ---------------------------------------------------------------------------
// Color enable / disable helpers
// ---------------------------------------------------------------------------

/// Returns `true` if color output should be used based on the current environment.
///
/// Returns `false` when:
/// - The `NO_COLOR` environment variable is set to any value (per <https://no-color.org/>)
/// - The `TERM` environment variable is `dumb`
pub fn should_use_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("TERM").ok().as_deref() == Some("dumb") {
        return false;
    }
    true
}

/// Returns whether color output is currently enabled.
///
/// This combines the environment check from [`should_use_color`] with any
/// runtime override applied via `--no-color`.  When `override_disabled` is
/// `true` the function unconditionally returns `false` *and* disables
/// colorization in the owo-colors subsystem for the remainder of the process.
pub fn color_enabled(override_disabled: bool) -> bool {
    let enabled = !override_disabled && should_use_color();
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
    if !enabled {
        owo_colors::set_override(false);
    }
    enabled
}

/// Query whether color output was enabled by the last call to [`color_enabled`].
pub fn is_color_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Compile-time Send + Sync check for palette constants
// ---------------------------------------------------------------------------

const _: () = {
    fn assert_send_sync<T: Send + Sync>() {}
    fn check() {
        assert_send_sync::<Style>();
    }
    let _ = check;
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_env_disables_color() {
        // Safety: single-threaded test manipulating env vars.
        // This test is not safe to run in parallel with other env-var tests,
        // but cargo test runs unit tests in a single binary where this is
        // the only writer, so it is acceptable.
        unsafe {
            std::env::set_var("NO_COLOR", "1");
        }
        let result = should_use_color();
        unsafe {
            std::env::remove_var("NO_COLOR");
        }
        assert!(!result, "should_use_color() must return false when NO_COLOR is set");
    }

    #[test]
    fn term_dumb_disables_color() {
        let old_term = std::env::var("TERM").ok();
        unsafe {
            std::env::set_var("TERM", "dumb");
        }
        let result = should_use_color();
        unsafe {
            match old_term {
                Some(v) => std::env::set_var("TERM", v),
                None => std::env::remove_var("TERM"),
            }
        }
        assert!(!result, "should_use_color() must return false when TERM=dumb");
    }

    #[test]
    fn style_for_deployment_type_covers_all_variants() {
        // Verify all DeploymentType variants map to the expected style constant.
        assert_eq!(
            format!("{:?}", style_for_deployment_type(DeploymentType::Proxy)),
            format!("{:?}", TYPE_PROXY),
        );
        assert_eq!(
            format!("{:?}", style_for_deployment_type(DeploymentType::Library)),
            format!("{:?}", TYPE_LIBRARY),
        );
        assert_eq!(
            format!("{:?}", style_for_deployment_type(DeploymentType::Singleton)),
            format!("{:?}", TYPE_SINGLETON),
        );
        assert_eq!(
            format!("{:?}", style_for_deployment_type(DeploymentType::Unknown)),
            format!("{:?}", TYPE_UNKNOWN),
        );
    }

    #[test]
    fn proxy_is_magenta_bold() {
        // Go: color.New(color.FgMagenta, color.Bold)
        let style = style_for_deployment_type(DeploymentType::Proxy);
        let expected = Style::new().magenta().bold();
        assert_eq!(format!("{:?}", style), format!("{:?}", expected));
    }

    #[test]
    fn library_is_blue_bold() {
        // Go: color.New(color.FgBlue, color.Bold)
        let style = style_for_deployment_type(DeploymentType::Library);
        let expected = Style::new().blue().bold();
        assert_eq!(format!("{:?}", style), format!("{:?}", expected));
    }

    #[test]
    fn verified_is_green_not_bold() {
        // Go: verifiedStyle = color.New(color.FgGreen)
        assert_eq!(format!("{:?}", VERIFIED), format!("{:?}", Style::new().green()));
    }

    #[test]
    fn not_verified_is_red_not_bold() {
        // Go: notVerifiedStyle = color.New(color.FgRed)
        assert_eq!(format!("{:?}", NOT_VERIFIED), format!("{:?}", Style::new().red()));
    }

    #[test]
    fn fork_indicator_is_yellow_not_bold() {
        // Go: forkIndicatorStyle = color.New(color.FgYellow)
        assert_eq!(format!("{:?}", FORK_INDICATOR), format!("{:?}", Style::new().yellow()));
    }

    #[test]
    fn address_is_white_not_dimmed() {
        // Go: addressStyle = color.New(color.FgWhite)
        assert_eq!(format!("{:?}", ADDRESS), format!("{:?}", Style::new().white()));
    }
}
