//! Broadcast file reading for historical deployment data.
//!
//! Reads forge broadcast JSON files to recover past deployment transactions
//! and contract addresses as a supplementary data source.

// TODO: Implement BroadcastData struct (sequence, chain_id, timestamp)
// TODO: Implement BroadcastTransaction struct (hash, contract_name, etc.)
// TODO: Implement read_latest_broadcast(config, contract_name, chain_id)
// TODO: Implement read_all_broadcasts(config, contract_name, chain_id)

/// Reader for forge broadcast files.
pub struct BroadcastReader;
