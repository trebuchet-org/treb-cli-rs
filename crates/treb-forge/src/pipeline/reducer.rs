//! Governance reducer invocation.
//!
//! A reducer is a forge Script that reads pending transactions from environment
//! variables and produces a single governance transaction via `vm.broadcast()`.
//! Rust captures the `BroadcastableTransactions` and routes them through the
//! proposer's sender chain.
//!
//! The built-in `GovernorReducer.sol` wraps pending txs into an OZ
//! `Governor.propose()` call. Custom reducers can target any governance system
//! by reading the same `TREB_REDUCER_*` env vars.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use alloy_primitives::{Address, U256};
use foundry_cheatcodes::BroadcastableTransactions;
use treb_core::error::TrebError;

/// Default path to the built-in OZ Governor reducer script.
const BUILTIN_GOVERNOR_REDUCER: &str = "lib/treb-sol/src/v2/reducers/GovernorReducer.sol";

/// RAII guard that sets environment variables on creation and removes them on drop.
struct ScopedEnvVars {
    keys: Vec<String>,
}

impl ScopedEnvVars {
    /// Set all `vars` in the process environment and return a guard that
    /// removes them when dropped.
    fn set(vars: HashMap<String, String>) -> Self {
        let keys: Vec<String> = vars.keys().cloned().collect();
        for (k, v) in &vars {
            // SAFETY: reducer invocation is single-threaded within the routing
            // pipeline — no concurrent env reads during script execution.
            unsafe { std::env::set_var(k, v); }
        }
        Self { keys }
    }
}

impl Drop for ScopedEnvVars {
    fn drop(&mut self) {
        for k in &self.keys {
            // SAFETY: see ScopedEnvVars::set
            unsafe { std::env::remove_var(k); }
        }
    }
}

/// Build the `TREB_REDUCER_*` environment variables for a reducer invocation.
pub fn build_reducer_env_vars(
    governor_address: Address,
    proposer_address: Address,
    targets: &[Address],
    values: &[U256],
    calldatas: &[Vec<u8>],
    description: &str,
    timelock_address: Option<Address>,
    chain_id: u64,
) -> HashMap<String, String> {
    use alloy_sol_types::SolValue;

    // ABI-encode (address[], uint256[], bytes[])
    let encoded = (
        targets.to_vec(),
        values.to_vec(),
        calldatas
            .iter()
            .map(|c| alloy_primitives::Bytes::from(c.clone()))
            .collect::<Vec<_>>(),
    )
        .abi_encode_params();

    let mut vars = HashMap::new();
    vars.insert(
        "TREB_REDUCER_ADDRESS".into(),
        format!("{:#x}", governor_address),
    );
    vars.insert(
        "TREB_REDUCER_PROPOSER".into(),
        format!("{:#x}", proposer_address),
    );
    vars.insert(
        "TREB_REDUCER_PENDING_TXS".into(),
        format!("0x{}", alloy_primitives::hex::encode(&encoded)),
    );
    vars.insert("TREB_REDUCER_DESCRIPTION".into(), description.to_string());
    vars.insert(
        "TREB_REDUCER_TIMELOCK".into(),
        timelock_address
            .map(|a| format!("{:#x}", a))
            .unwrap_or_default(),
    );
    vars.insert("TREB_REDUCER_CHAIN_ID".into(), chain_id.to_string());
    vars
}

/// Resolve the reducer script path for a Governor sender.
///
/// Resolution order:
/// 1. Explicit `reducer` field on the sender config → use that path
/// 2. No explicit reducer → built-in `lib/treb-sol/.../GovernorReducer.sol`
/// 3. Built-in path doesn't exist → `None` (caller falls back to inline Rust encoding)
pub fn resolve_reducer_path(
    reducer: Option<&str>,
    project_root: &Path,
) -> Option<PathBuf> {
    if let Some(explicit) = reducer {
        let p = project_root.join(explicit);
        if p.exists() {
            return Some(p);
        }
        // Explicit path that doesn't exist — warn but fall back
        eprintln!(
            "warning: reducer script not found: {}",
            p.display()
        );
        return None;
    }

    // Try built-in
    let builtin = project_root.join(BUILTIN_GOVERNOR_REDUCER);
    if builtin.exists() {
        return Some(builtin);
    }

    None
}

/// Invoke a reducer script and capture the resulting `BroadcastableTransactions`.
///
/// The reducer is executed as a forge script with `broadcast = false` so
/// transactions are captured but not sent. Environment variables are set via
/// [`ScopedEnvVars`] and cleaned up on return.
pub async fn invoke_reducer(
    reducer_path: &Path,
    env_vars: HashMap<String, String>,
    fork_url: &str,
    chain_id: u64,
) -> Result<BroadcastableTransactions, TrebError> {
    use crate::script::{ScriptConfig, execute_script};

    // Set TREB_REDUCER_* env vars for the duration of script execution
    let _guard = ScopedEnvVars::set(env_vars);

    let script_path = reducer_path
        .to_str()
        .ok_or_else(|| TrebError::Forge("reducer path is not valid UTF-8".into()))?;

    let mut config = ScriptConfig::new(script_path);
    config.fork_url(fork_url).chain_id(chain_id).broadcast(false);
    let args = config.into_script_args()?;

    let result = execute_script(args, None).await?;

    result.transactions.ok_or_else(|| {
        TrebError::Forge(
            "reducer script produced no transactions; \
             ensure the script calls vm.broadcast()"
                .into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_reducer_env_vars_encodes_all_fields() {
        use alloy_primitives::address;
        let gov = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let proposer = address!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let target = address!("cccccccccccccccccccccccccccccccccccccccc");
        let timelock = address!("dddddddddddddddddddddddddddddddddddddddd");

        let vars = build_reducer_env_vars(
            gov,
            proposer,
            &[target],
            &[U256::from(100)],
            &[vec![0x12, 0x34]],
            "test proposal",
            Some(timelock),
            42,
        );

        assert_eq!(vars["TREB_REDUCER_ADDRESS"], format!("{:#x}", gov));
        assert_eq!(vars["TREB_REDUCER_PROPOSER"], format!("{:#x}", proposer));
        assert!(vars["TREB_REDUCER_PENDING_TXS"].starts_with("0x"));
        assert_eq!(vars["TREB_REDUCER_DESCRIPTION"], "test proposal");
        assert_eq!(vars["TREB_REDUCER_TIMELOCK"], format!("{:#x}", timelock));
        assert_eq!(vars["TREB_REDUCER_CHAIN_ID"], "42");
    }

    #[test]
    fn build_reducer_env_vars_no_timelock() {
        let vars = build_reducer_env_vars(
            Address::ZERO,
            Address::ZERO,
            &[],
            &[],
            &[],
            "",
            None,
            1,
        );
        assert_eq!(vars["TREB_REDUCER_TIMELOCK"], "");
    }

    #[test]
    fn resolve_reducer_path_returns_none_for_missing_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(resolve_reducer_path(None, tmp.path()).is_none());
    }

    #[test]
    fn resolve_reducer_path_uses_explicit_when_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("MyReducer.sol");
        std::fs::write(&script, "// test").unwrap();
        let result = resolve_reducer_path(Some("MyReducer.sol"), tmp.path());
        assert_eq!(result, Some(script));
    }

    #[test]
    fn resolve_reducer_path_returns_none_for_missing_explicit() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(resolve_reducer_path(Some("missing.sol"), tmp.path()).is_none());
    }

    #[test]
    fn scoped_env_vars_cleanup() {
        let key = "TREB_TEST_SCOPED_ENV_12345";
        {
            let _guard = ScopedEnvVars::set(
                [(key.to_string(), "hello".to_string())]
                    .into_iter()
                    .collect(),
            );
            assert_eq!(std::env::var(key).unwrap(), "hello");
        }
        assert!(std::env::var(key).is_err());
    }
}
