//! Emoji constants matching the Go CLI render module.
//!
//! Every emoji character used across Go render files is defined here as a named
//! constant so that later phases can import them instead of scattering raw Unicode.

#![allow(dead_code)]

/// ✅ Success/completion (init, anvil, config, networks, tag, prune, generate)
pub const CHECK: &str = "✅";

/// ❌ Error/failure (prune, config, networks, anvil, init, compose)
pub const CROSS: &str = "❌";

/// ✓ Inline success (run, migrate, register, sync, compose, verify)
pub const CHECK_MARK: &str = "✓";

/// ✗ Inline failure (verify)
pub const CROSS_MARK: &str = "✗";

/// ⚠️ Warning indicator (helpers, prune, init, anvil, config, script)
pub const WARNING: &str = "⚠️";

/// 🚀 Deployment execution (transaction, script)
pub const ROCKET: &str = "🚀";

/// 🔄 Transactions section header (script), re-verifying (verify)
pub const REFRESH: &str = "🔄";

/// 🔁 Retry/partial verification (verify)
pub const REPEAT: &str = "🔁";

/// ⏳ Pending/queued status (deployments, verify)
pub const HOURGLASS: &str = "⏳";

/// ⏭️ Skip indicator (verify)
pub const FAST_FORWARD: &str = "⏭️";

/// 🆕 New verification status (verify)
pub const NEW: &str = "🆕";

/// 📦 Deployment summary (script, config, compose)
pub const PACKAGE: &str = "📦";

/// 📝 Script logs section (script)
pub const MEMO: &str = "📝";

/// 📋 List/plan (anvil, init, config, compose)
pub const CLIPBOARD: &str = "📋";

/// 📊 Statistics/status (anvil, compose)
pub const CHART: &str = "📊";

/// 📁 File location (config)
pub const FOLDER: &str = "📁";

/// 📂 Open folder / resume-from-state banner (compose)
pub const OPEN_FOLDER: &str = "📂";

/// 🎉 Celebration/success banner (init, compose)
pub const PARTY: &str = "🎉";

/// 🎯 Orchestration target (compose)
pub const TARGET: &str = "🎯";

/// 🌐 Network/RPC URL (anvil, networks)
pub const GLOBE: &str = "🌐";

/// 🔧 Maintenance/pruning (prune)
pub const WRENCH: &str = "🔧";

/// 🟢 Running status (anvil)
pub const GREEN_CIRCLE: &str = "🟢";

/// 🔴 Stopped status (anvil)
pub const RED_CIRCLE: &str = "🔴";

/// 🔍 Checking/searching (prune)
pub const MAGNIFYING_GLASS: &str = "🔍";

/// 🗑️ Items to delete (prune)
pub const WASTEBASKET: &str = "🗑️";

/// ✔︎ Verified status in deployment table (deployments)
pub const VERIFIED_WIDE: &str = "✔︎";

/// ◎ Namespace marker in deployment tree
pub const CIRCLE: &str = "◎";

/// ⛓ Chain marker in tree
pub const CHAIN_EMOJI: &str = "⛓";

/// 🏛️ Governor proposals section (script)
pub const CLASSICAL_BUILDING: &str = "🏛️";

#[cfg(test)]
mod tests {
    use super::CIRCLE;

    #[test]
    fn namespace_marker_matches_go_renderer() {
        assert_eq!(CIRCLE, "◎");
    }
}
