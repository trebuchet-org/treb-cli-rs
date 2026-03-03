//! Console log decoding for forge script output.
//!
//! Filters and decodes `console.log` messages from EVM execution logs.
//! Unrecognized log formats are silently skipped.

use alloy_primitives::Log;
use foundry_evm::core::decode::decode_console_logs as foundry_decode_console_logs;

/// Decode console.log messages from raw EVM logs.
///
/// Filters for the console.log precompile address and decodes all
/// recognized `console.log` variants. Unrecognized formats are silently
/// skipped (not errors).
pub fn decode_console_logs(logs: &[Log]) -> Vec<String> {
    foundry_decode_console_logs(logs)
}
