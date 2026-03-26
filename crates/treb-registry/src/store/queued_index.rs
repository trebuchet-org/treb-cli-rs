//! Persistent store for the active queued-work index backed by
//! `deployments/queued.json`.

use std::path::PathBuf;

use treb_core::TrebError;

use crate::{
    QUEUED_FILE,
    io::{read_versioned_file, write_versioned_file},
    types::QueuedIndex,
};

/// CRUD store for the active queued-work index.
pub struct QueuedIndexStore {
    path: PathBuf,
    data: QueuedIndex,
}

impl QueuedIndexStore {
    /// Create a new store pointing at `<deployments_dir>/queued.json`.
    pub fn new(deployments_dir: &std::path::Path) -> Self {
        Self { path: deployments_dir.join(QUEUED_FILE), data: QueuedIndex::default() }
    }

    /// Load the queued index from disk, replacing any in-memory data.
    pub fn load(&mut self) -> Result<(), TrebError> {
        self.data = read_versioned_file(&self.path)?;
        Ok(())
    }

    /// Persist the queued index to disk under a file lock.
    pub fn save(&self) -> Result<(), TrebError> {
        write_versioned_file(&self.path, &self.data)
    }

    /// Replace the full queued index and persist it atomically.
    pub fn replace_all(&mut self, data: QueuedIndex) -> Result<(), TrebError> {
        self.data = data;
        self.save()
    }

    /// Return the current queued index.
    pub fn data(&self) -> &QueuedIndex {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use treb_core::types::{ExecutionKind, ExecutionStatus};

    use super::*;

    #[test]
    fn replace_all_round_trips() {
        let dir = TempDir::new().unwrap();
        let mut store = QueuedIndexStore::new(dir.path());
        let index = QueuedIndex {
            entries: vec![crate::types::QueuedIndexEntry {
                deployment_ids: vec!["dep-1".into()],
                artifact_file: "broadcast/Deploy.s.sol/1/run-123.queued.json".into(),
                kind: ExecutionKind::SafeProposal,
                status: ExecutionStatus::Queued,
                tx_hash: None,
                safe_tx_hash: Some("0xsafe".into()),
                proposal_id: None,
                propose_safe_tx_hash: None,
            }],
        };

        store.replace_all(index.clone()).unwrap();

        let mut reloaded = QueuedIndexStore::new(dir.path());
        reloaded.load().unwrap();
        assert_eq!(reloaded.data(), &index);
    }
}
