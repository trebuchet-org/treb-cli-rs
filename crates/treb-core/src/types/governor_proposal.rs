//! Governor proposal model.
//!
//! Field names and serialization semantics match the Go implementation
//! at `treb-cli/internal/domain/models/governor_proposal.go`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::enums::ProposalStatus;

// ---------------------------------------------------------------------------
// GovernorProposal
// ---------------------------------------------------------------------------

/// A Governor proposal record for persistence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernorProposal {
    pub proposal_id: String,
    pub governor_address: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timelock_address: String,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub status: ProposalStatus,
    pub transaction_ids: Vec<String>,
    pub proposed_by: String,
    pub proposed_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub execution_tx_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_executed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<GovernorAction>,
}

// ---------------------------------------------------------------------------
// GovernorAction
// ---------------------------------------------------------------------------

/// A single action in a governance proposal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GovernorAction {
    pub target: String,
    pub value: String,
    pub calldata: String,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn sample_governor_proposal() -> GovernorProposal {
        GovernorProposal {
            proposal_id: "proposal-001".into(),
            governor_address: "0xGovernor".into(),
            timelock_address: String::new(),
            chain_id: 1,
            status: ProposalStatus::Pending,
            transaction_ids: vec!["tx-001".into()],
            proposed_by: "0xProposer".into(),
            proposed_at: Utc.with_ymd_and_hms(2025, 4, 1, 10, 0, 0).unwrap(),
            description: String::new(),
            executed_at: None,
            execution_tx_hash: String::new(),
            fork_executed_at: None,
            actions: Vec::new(),
        }
    }

    #[test]
    fn governor_proposal_camel_case_field_names() {
        let gp = sample_governor_proposal();
        let json = serde_json::to_value(&gp).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("proposalId"));
        assert!(obj.contains_key("governorAddress"));
        assert!(obj.contains_key("chainId"));
        assert!(obj.contains_key("status"));
        assert!(obj.contains_key("transactionIds"));
        assert!(obj.contains_key("proposedBy"));
        assert!(obj.contains_key("proposedAt"));

        // Verify no snake_case keys leaked
        assert!(!obj.contains_key("proposal_id"));
        assert!(!obj.contains_key("governor_address"));
        assert!(!obj.contains_key("chain_id"));
        assert!(!obj.contains_key("transaction_ids"));
        assert!(!obj.contains_key("proposed_by"));
        assert!(!obj.contains_key("proposed_at"));
        assert!(!obj.contains_key("timelock_address"));
        assert!(!obj.contains_key("executed_at"));
        assert!(!obj.contains_key("execution_tx_hash"));
    }

    #[test]
    fn omitempty_fields_omitted_when_empty() {
        let gp = sample_governor_proposal();
        let json = serde_json::to_value(&gp).unwrap();
        let obj = json.as_object().unwrap();

        assert!(
            !obj.contains_key("timelockAddress"),
            "timelockAddress should be omitted when empty"
        );
        assert!(!obj.contains_key("description"), "description should be omitted when empty");
        assert!(!obj.contains_key("executedAt"), "executedAt should be omitted when None");
        assert!(
            !obj.contains_key("executionTxHash"),
            "executionTxHash should be omitted when empty"
        );
    }

    #[test]
    fn omitempty_fields_present_when_populated() {
        let mut gp = sample_governor_proposal();
        gp.timelock_address = "0xTimelock".into();
        gp.description = "Upgrade to v2".into();
        gp.executed_at = Some(Utc.with_ymd_and_hms(2025, 4, 5, 14, 0, 0).unwrap());
        gp.execution_tx_hash = "0xexechash".into();

        let json = serde_json::to_value(&gp).unwrap();
        let obj = json.as_object().unwrap();

        assert_eq!(json["timelockAddress"], "0xTimelock");
        assert_eq!(json["description"], "Upgrade to v2");
        assert!(obj.contains_key("executedAt"));
        assert_eq!(json["executionTxHash"], "0xexechash");
    }

    #[test]
    fn transaction_ids_empty_array_not_null() {
        let mut gp = sample_governor_proposal();
        gp.transaction_ids = vec![];
        let json = serde_json::to_value(&gp).unwrap();
        assert_eq!(json["transactionIds"], serde_json::json!([]));
    }

    #[test]
    fn governor_proposal_serde_round_trip() {
        let gp = sample_governor_proposal();
        let json_str = serde_json::to_string_pretty(&gp).unwrap();
        let deserialized: GovernorProposal = serde_json::from_str(&json_str).unwrap();
        assert_eq!(gp, deserialized);
    }

    #[test]
    fn governor_proposal_with_execution_round_trip() {
        let mut gp = sample_governor_proposal();
        gp.status = ProposalStatus::Executed;
        gp.timelock_address = "0xTimelock".into();
        gp.description = "Upgrade to v2".into();
        gp.executed_at = Some(Utc.with_ymd_and_hms(2025, 4, 5, 14, 0, 0).unwrap());
        gp.execution_tx_hash = "0xexechash".into();

        let json_str = serde_json::to_string_pretty(&gp).unwrap();
        let deserialized: GovernorProposal = serde_json::from_str(&json_str).unwrap();
        assert_eq!(gp, deserialized);
    }
}
