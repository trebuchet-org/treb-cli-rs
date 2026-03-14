//! In-process Anvil node lifecycle management.
//!
//! Provides [`AnvilConfig`] (builder) and [`AnvilInstance`] (running node wrapper)
//! for spawning and managing Anvil instances programmatically without any subprocess calls.

use std::time::Duration;

use alloy_primitives::{Address, Bytes, U256};
use anvil::{AccountGenerator, NodeConfig, NodeHandle, eth::EthApi};
use tokio::task::AbortHandle;
use treb_core::error::TrebError;

// ---------------------------------------------------------------------------
// AnvilConfig
// ---------------------------------------------------------------------------

/// Configuration builder for an in-process Anvil node.
///
/// Use the builder methods to configure the node, then call [`spawn`](Self::spawn)
/// to start it.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), treb_core::error::TrebError> {
/// use treb_forge::anvil::AnvilConfig;
///
/// let instance = AnvilConfig::new()
///     .chain_id(31337)
///     .port(0) // OS-assigned port
///     .spawn()
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Default, Clone)]
pub struct AnvilConfig {
    chain_id: Option<u64>,
    fork_url: Option<String>,
    fork_block_number: Option<u64>,
    port: Option<u16>,
    accounts: Option<usize>,
    block_time: Option<Duration>,
}

impl AnvilConfig {
    /// Create a new [`AnvilConfig`] with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the EVM chain ID (default: 31337).
    pub fn chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = Some(chain_id);
        self
    }

    /// Set the upstream RPC URL to fork from.
    ///
    /// When set, Anvil will fork the chain at the specified block.
    pub fn fork_url(mut self, url: impl Into<String>) -> Self {
        self.fork_url = Some(url.into());
        self
    }

    /// Set the block number to fork from (requires [`fork_url`](Self::fork_url) to be set).
    ///
    /// If not set, forks from the latest block.
    pub fn fork_block_number(mut self, block: u64) -> Self {
        self.fork_block_number = Some(block);
        self
    }

    /// Set the HTTP RPC port.
    ///
    /// Use `0` to let the OS assign an available port (recommended for tests).
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Set the number of funded development accounts to generate (default: 10).
    pub fn accounts(mut self, accounts: usize) -> Self {
        self.accounts = Some(accounts);
        self
    }

    /// Set the interval between automatically mined blocks.
    ///
    /// If not set, blocks are mined on demand (after each transaction).
    pub fn block_time(mut self, block_time: Duration) -> Self {
        self.block_time = Some(block_time);
        self
    }

    /// Spawn an Anvil node with this configuration.
    ///
    /// Returns an [`AnvilInstance`] that manages the running node.
    /// Dropping the instance aborts the server tasks and frees the port.
    pub async fn spawn(self) -> Result<AnvilInstance, TrebError> {
        let mut config = NodeConfig::default().silent();

        if let Some(chain_id) = self.chain_id {
            config = config.with_chain_id(Some(chain_id));
        }

        if let Some(ref fork_url) = self.fork_url {
            config = config.with_eth_rpc_url(Some(fork_url.as_str()));
        }

        if let Some(block) = self.fork_block_number {
            config = config.with_fork_block_number(Some(block));
        }

        if let Some(port) = self.port {
            config = config.with_port(port);
        }

        if let Some(accounts) = self.accounts {
            let generator = AccountGenerator::new(accounts);
            config = config
                .with_account_generator(generator)
                .map_err(|e| TrebError::Fork(e.to_string()))?;
        }

        if let Some(block_time) = self.block_time {
            config = config.with_blocktime(Some(block_time));
        }

        let (api, handle) =
            anvil::try_spawn(config).await.map_err(|e| TrebError::Fork(e.to_string()))?;

        let rpc_url = handle.http_endpoint();
        let port = handle.socket_address().port();
        let chain_id = api.chain_id();

        // Collect abort handles from the public task fields so we can cancel them on drop.
        let node_abort = handle.node_service.abort_handle();
        let server_aborts: Vec<AbortHandle> =
            handle.servers.iter().map(|h| h.abort_handle()).collect();
        let mut abort_handles = vec![node_abort];
        abort_handles.extend(server_aborts);

        Ok(AnvilInstance {
            api,
            _handle: handle,
            _abort_handles: abort_handles,
            rpc_url,
            port,
            chain_id,
            fork_url: self.fork_url,
            fork_block_number: self.fork_block_number,
        })
    }
}

// ---------------------------------------------------------------------------
// AnvilInstance
// ---------------------------------------------------------------------------

/// A running in-process Anvil node.
///
/// Dropping this struct:
/// 1. Fires the graceful shutdown signal (via [`NodeHandle`]'s `Drop` impl).
/// 2. Aborts the underlying tokio tasks, releasing the listening port immediately.
pub struct AnvilInstance {
    api: EthApi,
    /// Held for its `Drop` impl which fires the graceful shutdown signal.
    _handle: NodeHandle,
    /// Abort handles for the server tasks — aborted before the handle is dropped.
    _abort_handles: Vec<AbortHandle>,
    rpc_url: String,
    port: u16,
    chain_id: u64,
    fork_url: Option<String>,
    fork_block_number: Option<u64>,
}

impl Drop for AnvilInstance {
    fn drop(&mut self) {
        // Explicitly abort the server tasks so the TcpListener is dropped and the port freed.
        // AbortHandle does not abort on drop — this call is required.
        for handle in &self._abort_handles {
            handle.abort();
        }
        // `_handle` (NodeHandle) is then dropped in field order, firing the graceful shutdown
        // signal.
    }
}

impl AnvilInstance {
    /// The HTTP RPC URL of this Anvil instance (e.g. `http://127.0.0.1:8545`).
    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    /// The port this Anvil instance is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The EVM chain ID of this Anvil instance.
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// The upstream fork URL, if this is a forked instance.
    pub fn fork_url(&self) -> Option<&str> {
        self.fork_url.as_deref()
    }

    /// The fork block number, if specified at configuration time.
    pub fn fork_block_number(&self) -> Option<u64> {
        self.fork_block_number
    }

    /// Create an EVM state snapshot and return its ID.
    ///
    /// The ID can later be passed to [`revert`](Self::revert) to restore this state.
    pub async fn snapshot(&self) -> Result<U256, TrebError> {
        self.api.evm_snapshot().await.map_err(|e| TrebError::Fork(e.to_string()))
    }

    /// Revert EVM state to a previously created snapshot.
    ///
    /// Returns `true` if the snapshot was found and applied.
    pub async fn revert(&self, snapshot_id: U256) -> Result<bool, TrebError> {
        self.api.evm_revert(snapshot_id).await.map_err(|e| TrebError::Fork(e.to_string()))
    }

    /// Set the bytecode stored at an address.
    ///
    /// Useful for injecting factory contracts (e.g. CreateX) at their canonical addresses.
    pub async fn set_code(&self, address: Address, code: Bytes) -> Result<(), TrebError> {
        self.api.anvil_set_code(address, code).await.map_err(|e| TrebError::Fork(e.to_string()))
    }

    /// Get the bytecode stored at an address.
    pub async fn get_code(&self, address: Address) -> Result<Bytes, TrebError> {
        self.api.get_code(address, None).await.map_err(|e| TrebError::Fork(e.to_string()))
    }

    /// Access the underlying [`EthApi`] for advanced operations not covered by this wrapper.
    pub fn api(&self) -> &EthApi {
        &self.api
    }
}

// ---------------------------------------------------------------------------
// Background subprocess Anvil — spawns `anvil` as a detached OS process
// ---------------------------------------------------------------------------

/// Information about a background Anvil subprocess spawned via [`spawn_background_anvil`].
#[derive(Debug)]
pub struct BackgroundAnvil {
    /// PID of the spawned Anvil process.
    pub pid: u32,
    /// Port Anvil is listening on.
    pub port: u16,
    /// Local RPC URL (e.g. `http://127.0.0.1:8545`).
    pub rpc_url: String,
    /// Path to the PID file.
    pub pid_file: std::path::PathBuf,
    /// Path to the log file.
    pub log_file: std::path::PathBuf,
}

/// Configuration for spawning a background Anvil subprocess.
#[derive(Debug, Clone)]
pub struct BackgroundAnvilConfig {
    /// Port to listen on.
    pub port: u16,
    /// Chain ID to set.
    pub chain_id: Option<u64>,
    /// Upstream RPC URL to fork from.
    pub fork_url: Option<String>,
    /// Block number to fork from.
    pub fork_block_number: Option<u64>,
    /// Path to the PID file.
    pub pid_file: std::path::PathBuf,
    /// Path to the log file.
    pub log_file: std::path::PathBuf,
}

/// Spawn Anvil as a detached background OS subprocess.
///
/// The process is started with `std::process::Command::spawn()` so it outlives
/// the current process. Stdout/stderr are redirected to the configured log file,
/// and the PID is written to the configured PID file.
///
/// After spawning, this function polls the RPC endpoint until healthy (up to 30
/// seconds for forked instances, 5 seconds otherwise).
pub fn spawn_background_anvil(
    config: &BackgroundAnvilConfig,
) -> Result<BackgroundAnvil, TrebError> {
    use std::fs;
    use std::process::{Command, Stdio};

    // Ensure parent directories exist for PID and log files.
    if let Some(parent) = config.pid_file.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| TrebError::Fork(format!("failed to create PID file directory: {e}")))?;
    }
    if let Some(parent) = config.log_file.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| TrebError::Fork(format!("failed to create log file directory: {e}")))?;
    }

    // Build args matching Go: --port, --host 0.0.0.0, --chain-id, --fork-url
    let mut args = vec![
        "--port".to_string(),
        config.port.to_string(),
        "--host".to_string(),
        "0.0.0.0".to_string(),
    ];
    if let Some(chain_id) = config.chain_id {
        args.push("--chain-id".to_string());
        args.push(chain_id.to_string());
    }
    if let Some(ref fork_url) = config.fork_url {
        args.push("--fork-url".to_string());
        args.push(fork_url.clone());
    }
    if let Some(block) = config.fork_block_number {
        args.push("--fork-block-number".to_string());
        args.push(block.to_string());
    }
    // Enable L2-specific features based on chain ID.
    // Note: --celo is intentionally omitted — it changes CREATE behavior
    // and causes FailedContractCreation with CreateX on forked Celo chains.
    if let Some(chain_id) = config.chain_id {
        match chain_id {
            10 | 420 | 8453 | 84532 | 7777777 | 999999999 => {
                args.push("--optimism".to_string())
            }
            _ => {}
        }
    }

    // Open log file for stdout/stderr redirection.
    let log_file = fs::File::create(&config.log_file)
        .map_err(|e| TrebError::Fork(format!("failed to create log file: {e}")))?;
    let log_file_err = log_file
        .try_clone()
        .map_err(|e| TrebError::Fork(format!("failed to clone log file handle: {e}")))?;

    let child = Command::new("anvil")
        .args(&args)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err))
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| TrebError::Fork(format!("failed to spawn anvil: {e}")))?;

    let pid = child.id();

    // Write PID file.
    fs::write(&config.pid_file, pid.to_string())
        .map_err(|e| TrebError::Fork(format!("failed to write PID file: {e}")))?;

    Ok(BackgroundAnvil {
        pid,
        port: config.port,
        rpc_url: format!("http://127.0.0.1:{}", config.port),
        pid_file: config.pid_file.clone(),
        log_file: config.log_file.clone(),
    })
}

/// Poll the RPC endpoint until it responds to `eth_blockNumber`, or until the
/// timeout elapses. Forked anvils can take longer to start (up to 30 s).
pub fn poll_anvil_health(rpc_url: &str, is_forked: bool) -> Result<(), TrebError> {
    use std::time::{Duration, Instant};

    let timeout = if is_forked { Duration::from_secs(30) } else { Duration::from_secs(5) };
    let interval = Duration::from_millis(200);
    let deadline = Instant::now() + timeout;

    let body = r#"{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}"#;

    loop {
        let result: Result<(), String> = (|| {
            let mut resp = ureq::post(rpc_url)
                .header("Content-Type", "application/json")
                .send(body.as_bytes())
                .map_err(|e| e.to_string())?;
            let buf = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
            if buf.contains("\"result\"") {
                Ok(())
            } else {
                Err(format!("unexpected response: {buf}"))
            }
        })();

        if result.is_ok() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            let err_msg = result.unwrap_err();
            return Err(TrebError::Fork(format!(
                "anvil not ready after {timeout:?}: {err_msg}"
            )));
        }

        std::thread::sleep(interval);
    }
}

/// Stop a background Anvil process by reading its PID file and sending SIGTERM.
///
/// This is safe to call even if the process is already dead. On failure, logs a
/// warning but does not return an error (matching Go behavior for cleanup paths).
pub fn stop_background_anvil(pid_file: &std::path::Path) -> Result<(), TrebError> {
    use std::fs;

    let pid_str = match fs::read_to_string(pid_file) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(TrebError::Fork(format!("failed to read PID file: {e}")));
        }
    };

    let pid: i32 = pid_str
        .trim()
        .parse()
        .map_err(|e| TrebError::Fork(format!("invalid PID in file: {e}")))?;

    // Send SIGTERM.
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with SIGTERM is safe — worst case pid doesn't exist (ESRCH).
        let ret = unsafe { libc::kill(pid, libc::SIGTERM) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            // ESRCH = no such process — already dead, that's fine.
            if err.raw_os_error() != Some(libc::ESRCH) {
                // Try SIGKILL as fallback.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }

        // Wait for process to exit (up to 5 seconds).
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let ret = unsafe { libc::kill(pid, 0) };
            if ret != 0 {
                break; // Process is gone.
            }
            if std::time::Instant::now() >= deadline {
                // Force kill.
                unsafe { libc::kill(pid, libc::SIGKILL) };
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    #[cfg(not(unix))]
    {
        // On non-Unix, use the `kill` command as a fallback.
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
    }

    // Remove PID file.
    let _ = fs::remove_file(pid_file);

    Ok(())
}

/// Find an available TCP port by binding to port 0 and reading back the assigned port.
pub fn find_available_port() -> Result<u16, TrebError> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| TrebError::Fork(format!("failed to find available port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| TrebError::Fork(format!("failed to read local address: {e}")))?
        .port();
    Ok(port)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use alloy_primitives::address;

    use super::*;

    /// Spawn a local devnet with OS-assigned port for test isolation.
    async fn test_instance() -> AnvilInstance {
        AnvilConfig::new().port(0).spawn().await.expect("spawn failed")
    }

    #[tokio::test]
    async fn spawn_returns_reachable_rpc_url() {
        let instance = test_instance().await;

        // rpc_url is well-formed
        assert!(instance.rpc_url().starts_with("http://127.0.0.1:"));
        assert!(instance.port() > 0);

        // Port is reachable
        let addr = format!("127.0.0.1:{}", instance.port());
        tokio::net::TcpStream::connect(&addr).await.expect("RPC port should be reachable");
    }

    #[tokio::test]
    async fn chain_id_config_works() {
        let instance = AnvilConfig::new().chain_id(42161).port(0).spawn().await.expect("spawn");
        assert_eq!(instance.chain_id(), 42161);
    }

    #[tokio::test]
    async fn snapshot_revert_round_trip() {
        let instance = test_instance().await;

        let test_addr = address!("1234567890123456789012345678901234567890");

        // Take a snapshot before any state change.
        let snap_id = instance.snapshot().await.expect("snapshot");

        // Modify state: give the test address some ETH.
        instance
            .api()
            .anvil_set_balance(test_addr, U256::from(1_000_000u64))
            .await
            .expect("set_balance");

        let balance_after_set =
            instance.api().balance(test_addr, None).await.expect("balance after set");
        assert_eq!(balance_after_set, U256::from(1_000_000u64));

        // Revert to the snapshot.
        let reverted = instance.revert(snap_id).await.expect("revert");
        assert!(reverted, "revert should return true");

        // Balance should be zero again (address never had ETH before snapshot).
        let balance_after_revert =
            instance.api().balance(test_addr, None).await.expect("balance after revert");
        assert_eq!(balance_after_revert, U256::ZERO, "balance should be zero after revert");
    }

    #[tokio::test]
    async fn set_code_verifiable_via_get_code() {
        let instance = test_instance().await;

        let target = address!("0000000000000000000000000000000000001234");
        let code = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0x56]); // dummy bytecode

        // Initially no code at the address.
        let before = instance.get_code(target).await.expect("get_code before");
        assert!(before.is_empty(), "no code expected at fresh address");

        // Set code.
        instance.set_code(target, code.clone()).await.expect("set_code");

        // Verify code was stored.
        let after = instance.get_code(target).await.expect("get_code after");
        assert_eq!(after, code, "stored code should match");
    }

    #[tokio::test]
    async fn dropping_instance_frees_port() {
        let port = {
            let instance = test_instance().await;
            let p = instance.port();

            // Verify port is reachable while alive.
            let addr = format!("127.0.0.1:{p}");
            tokio::net::TcpStream::connect(&addr).await.expect("port should be in use");
            p
            // instance dropped here: abort handles cancel tasks, NodeHandle drop fires signal
        };

        // Poll until the port is free or we time out (up to 2 seconds).
        let addr = format!("127.0.0.1:{port}");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if tokio::net::TcpStream::connect(&addr).await.is_err() {
                break; // Port is free — test passes.
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "port {port} still in use 2 seconds after dropping AnvilInstance"
            );
        }
    }
}
