//! EVM snapshot/revert utilities for pool release.
//!
//! Standalone helper functions that wrap [`AnvilNode`]'s snapshot/revert API
//! for use by the context pool to restore clean chain state between tests.

use std::collections::HashMap;

use alloy_primitives::U256;

use super::anvil_node::AnvilNode;

/// Take an EVM state snapshot of a single node.
///
/// Returns a snapshot ID that can later be passed to [`revert_snapshot`].
pub async fn take_snapshot(node: &AnvilNode) -> Result<U256, Box<dyn std::error::Error>> {
    let id = node.instance().snapshot().await?;
    Ok(id)
}

/// Revert a single node to a previously taken snapshot.
///
/// Returns an error if the underlying RPC call fails or if the snapshot ID
/// was not found (revert returned `false`).
pub async fn revert_snapshot(node: &AnvilNode, id: U256) -> Result<(), Box<dyn std::error::Error>> {
    let success = node.instance().revert(id).await?;
    if !success {
        return Err(format!("EVM revert failed for snapshot ID {id}").into());
    }
    Ok(())
}

/// Take EVM state snapshots of all named nodes.
///
/// Returns a map of node name → snapshot ID.
pub async fn take_snapshots(
    nodes: &HashMap<String, AnvilNode>,
) -> Result<HashMap<String, U256>, Box<dyn std::error::Error>> {
    let mut ids = HashMap::new();
    for (name, node) in nodes {
        let id = take_snapshot(node).await?;
        ids.insert(name.clone(), id);
    }
    Ok(ids)
}

/// Revert all named nodes to their previously saved snapshot IDs.
///
/// Errors if a node name in the map has no corresponding snapshot ID,
/// or if any individual revert fails.
pub async fn revert_snapshots(
    nodes: &HashMap<String, AnvilNode>,
    ids: &HashMap<String, U256>,
) -> Result<(), Box<dyn std::error::Error>> {
    for (name, node) in nodes {
        let id = ids
            .get(name)
            .ok_or_else(|| format!("no snapshot ID found for node '{name}'"))?;
        revert_snapshot(node, *id).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[tokio::test(flavor = "multi_thread")]
    async fn snapshot_modify_revert_restores_state() {
        let node = AnvilNode::spawn().await.expect("spawn failed");
        let test_addr = address!("1234567890123456789012345678901234567890");

        // Take snapshot of clean state.
        let snap_id = take_snapshot(&node).await.expect("take_snapshot");

        // Modify state: give the test address some ETH.
        node.instance()
            .api()
            .anvil_set_balance(test_addr, U256::from(999u64))
            .await
            .expect("set_balance");

        let balance = node
            .instance()
            .api()
            .balance(test_addr, None)
            .await
            .expect("balance");
        assert_eq!(balance, U256::from(999u64));

        // Revert to the snapshot.
        revert_snapshot(&node, snap_id)
            .await
            .expect("revert_snapshot");

        // Balance should be zero again.
        let balance_after = node
            .instance()
            .api()
            .balance(test_addr, None)
            .await
            .expect("balance after revert");
        assert_eq!(
            balance_after,
            U256::ZERO,
            "balance should be zero after revert"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn take_and_revert_snapshots_multi_node() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "node_a".to_string(),
            AnvilNode::spawn().await.expect("spawn node_a"),
        );
        nodes.insert(
            "node_b".to_string(),
            AnvilNode::spawn().await.expect("spawn node_b"),
        );

        let test_addr = address!("1234567890123456789012345678901234567890");

        // Take snapshots of all nodes.
        let snap_ids = take_snapshots(&nodes).await.expect("take_snapshots");
        assert_eq!(snap_ids.len(), 2);

        // Modify state on both nodes.
        for node in nodes.values() {
            node.instance()
                .api()
                .anvil_set_balance(test_addr, U256::from(42u64))
                .await
                .expect("set_balance");
        }

        // Revert all nodes.
        revert_snapshots(&nodes, &snap_ids)
            .await
            .expect("revert_snapshots");

        // Verify state restored on both.
        for node in nodes.values() {
            let balance = node
                .instance()
                .api()
                .balance(test_addr, None)
                .await
                .expect("balance after revert");
            assert_eq!(balance, U256::ZERO, "balance should be zero after revert");
        }
    }
}
