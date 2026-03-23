//! ContextPool — pre-warmed pool of TestContext instances with automatic cleanup.
//!
//! Provides [`ContextPool`] that pre-creates N [`TestContext`] instances with Anvil
//! nodes and offers thread-safe [`acquire`](ContextPool::acquire)/release with
//! automatic EVM snapshot revert and workspace cleanup.

use std::{
    collections::HashMap,
    ops::Deref,
    sync::{Arc, Mutex},
};

use alloy_primitives::U256;
use tokio::sync::Semaphore;
use treb_forge::anvil::AnvilConfig;

use super::{
    cleanup::clean_workspace,
    context::TestContext,
    snapshot::{revert_snapshots, take_snapshots},
};

/// An entry in the pool: a test context paired with its current snapshot IDs.
struct PoolEntry {
    context: TestContext,
    snapshot_ids: HashMap<String, U256>,
}

/// A pool of pre-warmed [`TestContext`] instances.
///
/// Each context comes with 2 Anvil nodes (chain IDs 31337 and 31338).
/// [`acquire`](Self::acquire) returns a [`PoolGuard`] that derefs to `TestContext`.
/// When the guard is dropped, EVM state is reverted, the workspace is cleaned,
/// fresh snapshots are taken, and the context is returned to the pool.
pub struct ContextPool {
    entries: Arc<Mutex<Vec<PoolEntry>>>,
    semaphore: Arc<Semaphore>,
}

impl ContextPool {
    /// Create a new pool with `size` pre-warmed contexts from the named fixture.
    ///
    /// Each context gets two Anvil nodes:
    /// - `"local"` with chain ID 31337 (port-mapped from 8545)
    /// - `"remote"` with chain ID 31338 (port-mapped from 9545)
    pub async fn new(size: usize, fixture_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut entries = Vec::with_capacity(size);

        for _ in 0..size {
            let ctx = TestContext::new(fixture_name)
                .with_anvil_mapped("local", AnvilConfig::new().chain_id(31337).port(0), 8545)
                .await?
                .with_anvil_mapped("remote", AnvilConfig::new().chain_id(31338).port(0), 9545)
                .await?;

            let snapshot_ids = take_snapshots(ctx.anvil_nodes()).await?;
            entries.push(PoolEntry { context: ctx, snapshot_ids });
        }

        Ok(Self {
            entries: Arc::new(Mutex::new(entries)),
            semaphore: Arc::new(Semaphore::new(size)),
        })
    }

    /// Acquire a context from the pool.
    ///
    /// Blocks if all contexts are currently in use.  The returned [`PoolGuard`]
    /// derefs to [`TestContext`] and automatically returns the context to the
    /// pool (with clean state) when dropped.
    pub async fn acquire(&self) -> PoolGuard {
        // Wait for a permit (blocks if pool is exhausted).
        let permit = self.semaphore.acquire().await.expect("semaphore closed");
        // Forget the permit — we manually add it back on release in Drop.
        permit.forget();

        let entry = self
            .entries
            .lock()
            .expect("pool lock poisoned")
            .pop()
            .expect("semaphore/pool count mismatch");

        PoolGuard {
            entry: Some(entry),
            entries: self.entries.clone(),
            semaphore: self.semaphore.clone(),
        }
    }

    /// Suggested pool size: reads `RAYON_NUM_THREADS` or falls back to CPU count.
    pub fn default_size() -> usize {
        std::env::var("RAYON_NUM_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(num_cpus::get)
    }
}

/// RAII guard that derefs to [`TestContext`] and returns it to the pool on drop.
///
/// When dropped:
/// 1. Reverts EVM snapshots on all Anvil nodes
/// 2. Cleans the workspace (removes build artifacts)
/// 3. Takes fresh snapshots for the next user
/// 4. Returns the context to the pool
pub struct PoolGuard {
    entry: Option<PoolEntry>,
    entries: Arc<Mutex<Vec<PoolEntry>>>,
    semaphore: Arc<Semaphore>,
}

impl Deref for PoolGuard {
    type Target = TestContext;

    fn deref(&self) -> &TestContext {
        &self.entry.as_ref().expect("guard already consumed").context
    }
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        if let Some(mut entry) = self.entry.take() {
            let entries = self.entries.clone();
            let semaphore = self.semaphore.clone();

            // Use block_in_place to run async cleanup inside synchronous Drop.
            // Requires tokio multi-thread runtime (which all our tests use).
            tokio::task::block_in_place(|| {
                let handle = tokio::runtime::Handle::current();
                handle.block_on(async {
                    // 1. Revert EVM snapshots.
                    let _ =
                        revert_snapshots(entry.context.anvil_nodes(), &entry.snapshot_ids).await;

                    // 2. Clean workspace.
                    clean_workspace(entry.context.path());

                    // 3. Take fresh snapshots for the next user.
                    if let Ok(ids) = take_snapshots(entry.context.anvil_nodes()).await {
                        entry.snapshot_ids = ids;
                    }

                    // 4. Return to pool and release permit.
                    entries.lock().expect("pool lock poisoned").push(entry);
                    semaphore.add_permits(1);
                });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[tokio::test(flavor = "multi_thread")]
    async fn acquire_release_clean_state() {
        let pool = match ContextPool::new(1, "minimal-project").await {
            Ok(pool) => pool,
            Err(err) if err.to_string().contains("Operation not permitted") => return,
            Err(err) => panic!("pool creation: {err}"),
        };

        let test_addr = address!("1234567890123456789012345678901234567890");

        // First acquire: modify chain state.
        {
            let guard = pool.acquire().await;
            let node = guard.anvil("local").expect("local node");
            node.instance().set_balance(test_addr, U256::from(999u64)).await.expect("set_balance");

            let balance = node.instance().balance(test_addr).await.expect("balance");
            assert_eq!(balance, U256::from(999u64));
            // guard dropped here — cleanup runs
        }

        // Second acquire: state should be clean after pool cleanup.
        {
            let guard = pool.acquire().await;
            let node = guard.anvil("local").expect("local node");

            let balance =
                node.instance().balance(test_addr).await.expect("balance after re-acquire");
            assert_eq!(balance, U256::ZERO, "balance should be zero after pool cleanup");
        }
    }
}
