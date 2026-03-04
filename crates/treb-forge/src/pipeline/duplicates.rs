//! Duplicate deployment detection and conflict resolution.
//!
//! Before recording deployments to the registry, the pipeline checks for
//! conflicts by deployment ID (exact match) and by address + chain_id.
//! The caller chooses a [`DuplicateStrategy`] to handle any detected conflicts.

use treb_core::{TrebError, types::deployment::Deployment};
use treb_registry::Registry;

use super::SkippedDeployment;

// ---------------------------------------------------------------------------
// ConflictType
// ---------------------------------------------------------------------------

/// The type of duplicate conflict detected.
#[derive(Debug, Clone, PartialEq)]
pub enum ConflictType {
    /// A deployment with the same ID already exists in the registry.
    SameId,
    /// A different deployment already exists at the same address + chain_id.
    SameAddress,
}

// ---------------------------------------------------------------------------
// DuplicateConflict
// ---------------------------------------------------------------------------

/// A conflict detected between a candidate deployment and an existing one.
#[derive(Debug, Clone)]
pub struct DuplicateConflict {
    /// What type of conflict was detected.
    pub conflict_type: ConflictType,
    /// The ID of the existing deployment that conflicts.
    pub existing_id: String,
}

// ---------------------------------------------------------------------------
// DuplicateStrategy
// ---------------------------------------------------------------------------

/// Strategy for handling duplicate deployments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DuplicateStrategy {
    /// Skip conflicting deployments and include them in the skipped list.
    Skip,
    /// Return an error on the first conflict.
    Error,
    /// Allow conflicting deployments through, marked for update.
    Update,
}

// ---------------------------------------------------------------------------
// ResolvedDuplicates
// ---------------------------------------------------------------------------

/// Result of resolving duplicates for a set of candidate deployments.
#[derive(Debug)]
pub struct ResolvedDuplicates {
    /// Deployments with no conflict that should be inserted.
    pub to_insert: Vec<Deployment>,
    /// Deployments with a conflict that should be updated (Update strategy).
    pub to_update: Vec<Deployment>,
    /// Deployments that were skipped due to conflicts (Skip strategy).
    pub skipped: Vec<SkippedDeployment>,
}

// ---------------------------------------------------------------------------
// check_duplicate
// ---------------------------------------------------------------------------

/// Check whether a candidate deployment conflicts with an existing one.
///
/// Checks by ID first (takes precedence), then by address + chain_id.
/// Returns `None` when no conflict is found.
pub fn check_duplicate(candidate: &Deployment, registry: &Registry) -> Option<DuplicateConflict> {
    // SameId takes precedence.
    if registry.get_deployment(&candidate.id).is_some() {
        return Some(DuplicateConflict {
            conflict_type: ConflictType::SameId,
            existing_id: candidate.id.clone(),
        });
    }

    // SameAddress: same address + chain_id (skip empty addresses).
    if !candidate.address.is_empty() {
        for existing in registry.list_deployments() {
            if existing.address.eq_ignore_ascii_case(&candidate.address)
                && existing.chain_id == candidate.chain_id
            {
                return Some(DuplicateConflict {
                    conflict_type: ConflictType::SameAddress,
                    existing_id: existing.id.clone(),
                });
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// resolve_duplicates
// ---------------------------------------------------------------------------

/// Resolve duplicate conflicts for a batch of candidate deployments.
///
/// Depending on the chosen [`DuplicateStrategy`]:
/// - **Skip**: conflicting candidates are moved to `skipped` with a reason.
/// - **Error**: returns an error on the first conflict.
/// - **Update**: conflicting candidates are moved to `to_update`.
///
/// Non-conflicting candidates always land in `to_insert`.
pub fn resolve_duplicates(
    candidates: Vec<Deployment>,
    registry: &Registry,
    strategy: DuplicateStrategy,
) -> Result<ResolvedDuplicates, TrebError> {
    let mut to_insert = Vec::new();
    let mut to_update = Vec::new();
    let mut skipped = Vec::new();

    for candidate in candidates {
        match check_duplicate(&candidate, registry) {
            None => to_insert.push(candidate),
            Some(conflict) => match strategy {
                DuplicateStrategy::Skip => {
                    let reason = match conflict.conflict_type {
                        ConflictType::SameId => {
                            format!("Deployment with ID '{}' already exists", conflict.existing_id)
                        }
                        ConflictType::SameAddress => format!(
                            "Address {} on chain {} already registered as '{}'",
                            candidate.address, candidate.chain_id, conflict.existing_id
                        ),
                    };
                    skipped.push(SkippedDeployment { deployment: candidate, reason });
                }
                DuplicateStrategy::Error => {
                    let msg = match conflict.conflict_type {
                        ConflictType::SameId => format!(
                            "Duplicate deployment: ID '{}' already exists",
                            conflict.existing_id
                        ),
                        ConflictType::SameAddress => format!(
                            "Duplicate deployment: address {} on chain {} already registered as '{}'",
                            candidate.address, candidate.chain_id, conflict.existing_id
                        ),
                    };
                    return Err(TrebError::Registry(msg));
                }
                DuplicateStrategy::Update => {
                    to_update.push(candidate);
                }
            },
        }
    }

    Ok(ResolvedDuplicates { to_insert, to_update, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use chrono::Utc;
    use tempfile::TempDir;
    use treb_core::types::{
        deployment::{ArtifactInfo, DeploymentStrategy, VerificationInfo},
        enums::{DeploymentMethod, DeploymentType, VerificationStatus},
    };

    /// Build a minimal Deployment with configurable id, address, and chain_id.
    fn make_deployment(id: &str, address: &str, chain_id: u64) -> Deployment {
        let now = Utc::now();
        Deployment {
            id: id.to_string(),
            namespace: "production".to_string(),
            chain_id,
            contract_name: "Counter".to_string(),
            label: "v1".to_string(),
            address: address.to_string(),
            deployment_type: DeploymentType::Singleton,
            transaction_id: String::new(),
            deployment_strategy: DeploymentStrategy {
                method: DeploymentMethod::Create,
                salt: String::new(),
                init_code_hash: String::new(),
                factory: String::new(),
                constructor_args: String::new(),
                entropy: String::new(),
            },
            proxy_info: None,
            artifact: ArtifactInfo {
                path: String::new(),
                compiler_version: String::new(),
                bytecode_hash: String::new(),
                script_path: String::new(),
                git_commit: String::new(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create an initialized registry in a temp dir with pre-seeded deployments.
    fn registry_with(deployments: Vec<Deployment>) -> (TempDir, Registry) {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();
        for dep in deployments {
            registry.insert_deployment(dep).unwrap();
        }
        (dir, registry)
    }

    // ── check_duplicate tests ───────────────────────────────────────────

    #[test]
    fn no_duplicates_all_candidates_pass_through() {
        let (_dir, registry) =
            registry_with(vec![make_deployment("existing/1/Token:v1", "0xAAA", 1)]);

        // Candidate has different ID and different address
        let candidate = make_deployment("production/1/Counter:v1", "0xBBB", 1);
        assert!(check_duplicate(&candidate, &registry).is_none());
    }

    #[test]
    fn duplicate_by_id_correctly_detected() {
        let (_dir, registry) =
            registry_with(vec![make_deployment("production/1/Counter:v1", "0xAAA", 1)]);

        let candidate = make_deployment("production/1/Counter:v1", "0xBBB", 1);
        let conflict = check_duplicate(&candidate, &registry).expect("should detect conflict");
        assert_eq!(conflict.conflict_type, ConflictType::SameId);
        assert_eq!(conflict.existing_id, "production/1/Counter:v1");
    }

    #[test]
    fn duplicate_by_address_chain_correctly_detected() {
        let (_dir, registry) =
            registry_with(vec![make_deployment("existing/1/Token:v1", "0xAAA", 1)]);

        // Different ID but same address + chain_id
        let candidate = make_deployment("production/1/Counter:v1", "0xAAA", 1);
        let conflict = check_duplicate(&candidate, &registry).expect("should detect conflict");
        assert_eq!(conflict.conflict_type, ConflictType::SameAddress);
        assert_eq!(conflict.existing_id, "existing/1/Token:v1");
    }

    #[test]
    fn same_id_takes_precedence_over_same_address() {
        let (_dir, registry) =
            registry_with(vec![make_deployment("production/1/Counter:v1", "0xAAA", 1)]);

        // Both same ID and same address
        let candidate = make_deployment("production/1/Counter:v1", "0xAAA", 1);
        let conflict = check_duplicate(&candidate, &registry).expect("should detect conflict");
        assert_eq!(
            conflict.conflict_type,
            ConflictType::SameId,
            "SameId should take precedence over SameAddress"
        );
    }

    #[test]
    fn same_address_different_chain_not_a_conflict() {
        let (_dir, registry) =
            registry_with(vec![make_deployment("existing/1/Token:v1", "0xAAA", 1)]);

        // Same address but different chain_id
        let candidate = make_deployment("production/42/Counter:v1", "0xAAA", 42);
        assert!(
            check_duplicate(&candidate, &registry).is_none(),
            "same address on a different chain should not conflict"
        );
    }

    // ── resolve_duplicates tests ────────────────────────────────────────

    #[test]
    fn resolve_no_duplicates_all_pass_through() {
        let (_dir, registry) = registry_with(vec![]);

        let candidates = vec![
            make_deployment("production/1/Counter:v1", "0xAAA", 1),
            make_deployment("production/1/Token:v1", "0xBBB", 1),
        ];

        let result = resolve_duplicates(candidates, &registry, DuplicateStrategy::Skip).unwrap();
        assert_eq!(result.to_insert.len(), 2);
        assert!(result.to_update.is_empty());
        assert!(result.skipped.is_empty());
    }

    #[test]
    fn resolve_skip_strategy_skips_with_reason() {
        let (_dir, registry) =
            registry_with(vec![make_deployment("production/1/Counter:v1", "0xAAA", 1)]);

        let candidates = vec![
            make_deployment("production/1/Counter:v1", "0xBBB", 1), // same ID
            make_deployment("production/1/Token:v1", "0xCCC", 1),   // no conflict
        ];

        let result = resolve_duplicates(candidates, &registry, DuplicateStrategy::Skip).unwrap();
        assert_eq!(result.to_insert.len(), 1);
        assert_eq!(result.to_insert[0].id, "production/1/Token:v1");
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].deployment.id, "production/1/Counter:v1");
        assert!(
            result.skipped[0].reason.contains("already exists"),
            "reason should explain the conflict: {}",
            result.skipped[0].reason
        );
    }

    #[test]
    fn resolve_error_strategy_returns_error_on_conflict() {
        let (_dir, registry) =
            registry_with(vec![make_deployment("production/1/Counter:v1", "0xAAA", 1)]);

        let candidates = vec![
            make_deployment("production/1/Counter:v1", "0xBBB", 1), // conflict
            make_deployment("production/1/Token:v1", "0xCCC", 1),   // would be fine
        ];

        let result = resolve_duplicates(candidates, &registry, DuplicateStrategy::Error);
        assert!(result.is_err(), "Error strategy should return Err on conflict");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Duplicate deployment"),
            "error message should be descriptive: {err_msg}"
        );
    }

    #[test]
    fn resolve_update_strategy_marks_for_update() {
        let (_dir, registry) =
            registry_with(vec![make_deployment("production/1/Counter:v1", "0xAAA", 1)]);

        let candidates = vec![
            make_deployment("production/1/Counter:v1", "0xBBB", 1), // conflict → update
            make_deployment("production/1/Token:v1", "0xCCC", 1),   // no conflict → insert
        ];

        let result = resolve_duplicates(candidates, &registry, DuplicateStrategy::Update).unwrap();
        assert_eq!(result.to_insert.len(), 1);
        assert_eq!(result.to_insert[0].id, "production/1/Token:v1");
        assert_eq!(result.to_update.len(), 1);
        assert_eq!(result.to_update[0].id, "production/1/Counter:v1");
        assert!(result.skipped.is_empty());
    }
}
