//! Persistent store for governor proposals backed by `governor_proposals.json`.

use std::collections::HashMap;
use std::path::PathBuf;

use treb_core::types::GovernorProposal;
use treb_core::TrebError;

use crate::io::{read_json_file_or_default, with_file_lock, write_json_file};
use crate::GOVERNOR_PROPOSALS_FILE;

/// CRUD store for governor proposals, persisted as a
/// `HashMap<String, GovernorProposal>` in `governor_proposals.json` inside the
/// registry directory. Keyed by `proposal_id`.
pub struct GovernorProposalStore {
    path: PathBuf,
    data: HashMap<String, GovernorProposal>,
}

impl GovernorProposalStore {
    /// Create a new store pointing at `<registry_dir>/governor_proposals.json`.
    /// Call [`load`](Self::load) to read existing data from disk.
    pub fn new(registry_dir: &std::path::Path) -> Self {
        Self {
            path: registry_dir.join(GOVERNOR_PROPOSALS_FILE),
            data: HashMap::new(),
        }
    }

    /// Load governor proposals from disk, replacing any in-memory data.
    pub fn load(&mut self) -> Result<(), TrebError> {
        self.data = read_json_file_or_default(&self.path)?;
        Ok(())
    }

    /// Atomically save all governor proposals to disk under a file lock.
    pub fn save(&self) -> Result<(), TrebError> {
        with_file_lock(&self.path, || write_json_file(&self.path, &self.data))
    }

    /// Get a governor proposal by its `proposal_id`.
    pub fn get(&self, proposal_id: &str) -> Option<&GovernorProposal> {
        self.data.get(proposal_id)
    }

    /// Insert a new governor proposal. Returns an error if the ID already exists.
    pub fn insert(&mut self, proposal: GovernorProposal) -> Result<(), TrebError> {
        if self.data.contains_key(&proposal.proposal_id) {
            return Err(TrebError::Registry(format!(
                "governor proposal already exists: {}",
                proposal.proposal_id
            )));
        }
        self.data
            .insert(proposal.proposal_id.clone(), proposal);
        self.save()
    }

    /// Update an existing governor proposal.
    /// Returns an error if the ID is not found.
    pub fn update(&mut self, proposal: GovernorProposal) -> Result<(), TrebError> {
        if !self.data.contains_key(&proposal.proposal_id) {
            return Err(TrebError::Registry(format!(
                "governor proposal not found: {}",
                proposal.proposal_id
            )));
        }
        self.data
            .insert(proposal.proposal_id.clone(), proposal);
        self.save()
    }

    /// Remove a governor proposal by ID, returning it if found.
    pub fn remove(&mut self, proposal_id: &str) -> Result<GovernorProposal, TrebError> {
        let proposal = self.data.remove(proposal_id).ok_or_else(|| {
            TrebError::Registry(format!("governor proposal not found: {proposal_id}"))
        })?;
        self.save()?;
        Ok(proposal)
    }

    /// List all governor proposals sorted by `proposed_at` (descending).
    pub fn list(&self) -> Vec<&GovernorProposal> {
        let mut entries: Vec<&GovernorProposal> = self.data.values().collect();
        entries.sort_by(|a, b| b.proposed_at.cmp(&a.proposed_at));
        entries
    }

    /// Return the number of governor proposals in the store.
    pub fn count(&self) -> usize {
        self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;
    use treb_core::types::ProposalStatus;

    /// Helper to create a minimal governor proposal with the given ID and
    /// proposed_at offset in seconds.
    fn make_governor_proposal(id: &str, proposed_at_offset_secs: i64) -> GovernorProposal {
        let base = chrono::DateTime::parse_from_rfc3339("2026-03-02T19:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts = base + chrono::Duration::seconds(proposed_at_offset_secs);
        GovernorProposal {
            proposal_id: id.to_string(),
            governor_address: "0xGovernor".to_string(),
            timelock_address: String::new(),
            chain_id: 1,
            status: ProposalStatus::Pending,
            transaction_ids: vec![],
            proposed_by: "0xProposer".to_string(),
            proposed_at: ts,
            description: String::new(),
            executed_at: None,
            execution_tx_hash: String::new(),
        }
    }

    #[test]
    fn insert_then_get() {
        let dir = TempDir::new().unwrap();
        let mut store = GovernorProposalStore::new(dir.path());

        let proposal = make_governor_proposal("prop-1", 0);
        store.insert(proposal.clone()).unwrap();

        let got = store.get("prop-1").unwrap();
        assert_eq!(got.proposal_id, "prop-1");
        assert_eq!(got.chain_id, 1);
    }

    #[test]
    fn duplicate_insert_error() {
        let dir = TempDir::new().unwrap();
        let mut store = GovernorProposalStore::new(dir.path());

        let proposal = make_governor_proposal("prop-1", 0);
        store.insert(proposal.clone()).unwrap();

        let result = store.insert(proposal);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already exists"),
            "expected 'already exists' in: {msg}"
        );
    }

    #[test]
    fn update_success() {
        let dir = TempDir::new().unwrap();
        let mut store = GovernorProposalStore::new(dir.path());

        let proposal = make_governor_proposal("prop-1", 0);
        store.insert(proposal.clone()).unwrap();

        let mut modified = proposal;
        modified.status = ProposalStatus::Executed;
        store.update(modified).unwrap();

        let got = store.get("prop-1").unwrap();
        assert_eq!(got.status, ProposalStatus::Executed);
    }

    #[test]
    fn update_nonexistent_error() {
        let dir = TempDir::new().unwrap();
        let mut store = GovernorProposalStore::new(dir.path());

        let proposal = make_governor_proposal("prop-missing", 0);
        let result = store.update(proposal);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not found"),
            "expected 'not found' in: {msg}"
        );
    }

    #[test]
    fn remove_returns_proposal() {
        let dir = TempDir::new().unwrap();
        let mut store = GovernorProposalStore::new(dir.path());

        let proposal = make_governor_proposal("prop-1", 0);
        store.insert(proposal).unwrap();

        let removed = store.remove("prop-1").unwrap();
        assert_eq!(removed.proposal_id, "prop-1");
        assert!(store.get("prop-1").is_none());
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn list_sorted_by_proposed_at_descending() {
        let dir = TempDir::new().unwrap();
        let mut store = GovernorProposalStore::new(dir.path());

        // Insert in ascending order
        store
            .insert(make_governor_proposal("prop-1", 10))
            .unwrap();
        store
            .insert(make_governor_proposal("prop-2", 20))
            .unwrap();
        store
            .insert(make_governor_proposal("prop-3", 30))
            .unwrap();

        let list = store.list();
        assert_eq!(list.len(), 3);
        // Descending: newest first
        assert_eq!(list[0].proposal_id, "prop-3");
        assert_eq!(list[1].proposal_id, "prop-2");
        assert_eq!(list[2].proposal_id, "prop-1");
    }

    #[test]
    fn empty_store_operations() {
        let dir = TempDir::new().unwrap();
        let store = GovernorProposalStore::new(dir.path());

        assert_eq!(store.count(), 0);
        assert!(store.list().is_empty());
        assert!(store.get("anything").is_none());
    }

    #[test]
    fn round_trip_persistence() {
        let dir = TempDir::new().unwrap();

        // Insert proposals via first store instance
        {
            let mut store = GovernorProposalStore::new(dir.path());
            store
                .insert(make_governor_proposal("prop-1", 10))
                .unwrap();
            store
                .insert(make_governor_proposal("prop-2", 20))
                .unwrap();
        }

        // Load from disk via second store instance
        let mut store2 = GovernorProposalStore::new(dir.path());
        store2.load().unwrap();

        assert_eq!(store2.count(), 2);
        assert!(store2.get("prop-1").is_some());
        assert!(store2.get("prop-2").is_some());
    }

    #[test]
    fn count_correctness() {
        let dir = TempDir::new().unwrap();
        let mut store = GovernorProposalStore::new(dir.path());

        assert_eq!(store.count(), 0);
        store
            .insert(make_governor_proposal("prop-1", 10))
            .unwrap();
        assert_eq!(store.count(), 1);
        store
            .insert(make_governor_proposal("prop-2", 20))
            .unwrap();
        assert_eq!(store.count(), 2);
        store.remove("prop-1").unwrap();
        assert_eq!(store.count(), 1);
    }
}
