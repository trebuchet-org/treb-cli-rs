//! Proxy relationship detection from ERC-1967 events.
//!
//! Analyses decoded [`ProxyEvent`] entries to determine proxy types and
//! link proxy contracts to their corresponding deployments.

use std::collections::HashMap;

use alloy_primitives::Address;

use crate::events::{
    decoder::{ParsedEvent, ProxyEvent},
    deployments::ExtractedDeployment,
};

/// The type of proxy contract detected from ERC-1967 events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyType {
    /// Transparent proxy — emits both `Upgraded` and `AdminChanged`.
    Transparent,
    /// UUPS proxy — emits only `Upgraded` (no admin change).
    UUPS,
    /// Beacon proxy — emits `BeaconUpgraded`.
    Beacon,
    /// Minimal proxy (EIP-1167 clone) — detected by bytecode analysis,
    /// not by events. Reserved for future use.
    Minimal,
}

/// A detected proxy relationship linking a proxy to its implementation.
#[derive(Debug, Clone)]
pub struct ProxyRelationship {
    /// The proxy contract address.
    pub proxy_address: Address,
    /// The detected proxy type.
    pub proxy_type: ProxyType,
    /// The implementation contract address (set by `Upgraded` or looked up
    /// via beacon).
    pub implementation: Option<Address>,
    /// The proxy admin address (only for [`ProxyType::Transparent`]).
    pub admin: Option<Address>,
    /// The beacon address (only for [`ProxyType::Beacon`]).
    pub beacon: Option<Address>,
}

/// Detect proxy relationships from decoded events.
///
/// Iterates through `events` collecting [`ProxyEvent`] entries, grouping
/// them by emitting address. The proxy type is inferred from the
/// combination of events seen:
///
/// - `Upgraded` only → [`ProxyType::UUPS`]
/// - `Upgraded` + `AdminChanged` from the same address → [`ProxyType::Transparent`]
/// - `BeaconUpgraded` → [`ProxyType::Beacon`]
pub fn detect_proxy_relationships(events: &[ParsedEvent]) -> HashMap<Address, ProxyRelationship> {
    let mut relationships: HashMap<Address, ProxyRelationship> = HashMap::new();

    for event in events {
        if let ParsedEvent::Proxy(proxy_event) = event {
            match proxy_event {
                ProxyEvent::Upgraded { proxy_address, implementation } => {
                    let entry =
                        relationships.entry(*proxy_address).or_insert_with(|| ProxyRelationship {
                            proxy_address: *proxy_address,
                            proxy_type: ProxyType::UUPS,
                            implementation: None,
                            admin: None,
                            beacon: None,
                        });
                    entry.implementation = Some(*implementation);
                    // Don't overwrite Transparent if already detected.
                }
                ProxyEvent::AdminChanged { proxy_address, new_admin, .. } => {
                    let entry =
                        relationships.entry(*proxy_address).or_insert_with(|| ProxyRelationship {
                            proxy_address: *proxy_address,
                            proxy_type: ProxyType::Transparent,
                            implementation: None,
                            admin: None,
                            beacon: None,
                        });
                    // AdminChanged means this is a Transparent proxy.
                    entry.proxy_type = ProxyType::Transparent;
                    entry.admin = Some(*new_admin);
                }
                ProxyEvent::BeaconUpgraded { proxy_address, beacon } => {
                    let entry =
                        relationships.entry(*proxy_address).or_insert_with(|| ProxyRelationship {
                            proxy_address: *proxy_address,
                            proxy_type: ProxyType::Beacon,
                            implementation: None,
                            admin: None,
                            beacon: None,
                        });
                    entry.proxy_type = ProxyType::Beacon;
                    entry.beacon = Some(*beacon);
                }
            }
        }
    }

    relationships
}

/// Link a proxy relationship to the deployment that created the proxy contract.
///
/// Searches `deployments` for one whose `address` matches the proxy's
/// `proxy_address`. Returns `Some` index into the deployments slice if found.
pub fn link_proxy_to_deployment(
    proxy: &ProxyRelationship,
    deployments: &[ExtractedDeployment],
) -> Option<usize> {
    deployments.iter().position(|d| d.address == proxy.proxy_address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{B256, Bytes, LogData, address};
    use alloy_sol_types::SolEvent;

    use crate::events::{
        abi::{AdminChanged, BeaconUpgraded, ContractDeployed, DeploymentDetails, Upgraded},
        decoder::decode_events,
        deployments::extract_deployments,
    };

    /// Helper: create a raw Log from an event type's encoded log data.
    fn make_log(address: Address, log_data: LogData) -> alloy_primitives::Log {
        alloy_primitives::Log::new(address, log_data.topics().to_vec(), log_data.data.clone())
            .expect("valid log data")
    }

    fn sample_deployed_event(location: Address, label: &str) -> ContractDeployed {
        ContractDeployed {
            deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            location,
            transactionId: B256::ZERO,
            deployment: DeploymentDetails {
                artifact: "MyContract".to_string(),
                label: label.to_string(),
                entropy: String::new(),
                salt: B256::ZERO,
                bytecodeHash: B256::ZERO,
                initCodeHash: B256::ZERO,
                constructorArgs: Bytes::new(),
                createStrategy: "create".to_string(),
            },
        }
    }

    #[test]
    fn single_uups_proxy() {
        let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let impl_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");

        let upgraded = Upgraded { implementation: impl_addr };
        let log = make_log(proxy_addr, upgraded.encode_log_data());
        let parsed = decode_events(&[log]);

        let relationships = detect_proxy_relationships(&parsed);
        assert_eq!(relationships.len(), 1);

        let rel = relationships.get(&proxy_addr).expect("proxy should exist");
        assert_eq!(rel.proxy_type, ProxyType::UUPS);
        assert_eq!(rel.proxy_address, proxy_addr);
        assert_eq!(rel.implementation, Some(impl_addr));
        assert!(rel.admin.is_none());
        assert!(rel.beacon.is_none());
    }

    #[test]
    fn transparent_proxy_upgraded_and_admin_changed() {
        let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let impl_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
        let admin_addr = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");

        let upgraded = Upgraded { implementation: impl_addr };
        let admin_changed = AdminChanged { previousAdmin: Address::ZERO, newAdmin: admin_addr };

        let log1 = make_log(proxy_addr, upgraded.encode_log_data());
        let log2 = make_log(proxy_addr, admin_changed.encode_log_data());
        let parsed = decode_events(&[log1, log2]);

        let relationships = detect_proxy_relationships(&parsed);
        assert_eq!(relationships.len(), 1);

        let rel = relationships.get(&proxy_addr).expect("proxy should exist");
        assert_eq!(rel.proxy_type, ProxyType::Transparent);
        assert_eq!(rel.implementation, Some(impl_addr));
        assert_eq!(rel.admin, Some(admin_addr));
        assert!(rel.beacon.is_none());
    }

    #[test]
    fn beacon_proxy() {
        let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let beacon_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");

        let beacon_upgraded = BeaconUpgraded { beacon: beacon_addr };
        let log = make_log(proxy_addr, beacon_upgraded.encode_log_data());
        let parsed = decode_events(&[log]);

        let relationships = detect_proxy_relationships(&parsed);
        assert_eq!(relationships.len(), 1);

        let rel = relationships.get(&proxy_addr).expect("proxy should exist");
        assert_eq!(rel.proxy_type, ProxyType::Beacon);
        assert_eq!(rel.beacon, Some(beacon_addr));
        assert!(rel.implementation.is_none());
        assert!(rel.admin.is_none());
    }

    #[test]
    fn multiple_proxies_tracked_independently() {
        let proxy1 = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let proxy2 = address!("CafaC3bA6F55205a7cf03AB48fBaD8d0e3F0F07E");
        let impl1 = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
        let beacon_addr = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");

        // Proxy 1: UUPS (Upgraded only)
        let upgraded1 = Upgraded { implementation: impl1 };
        let log1 = make_log(proxy1, upgraded1.encode_log_data());

        // Proxy 2: Beacon
        let beacon_upgraded = BeaconUpgraded { beacon: beacon_addr };
        let log2 = make_log(proxy2, beacon_upgraded.encode_log_data());

        let parsed = decode_events(&[log1, log2]);
        let relationships = detect_proxy_relationships(&parsed);
        assert_eq!(relationships.len(), 2);

        let rel1 = relationships.get(&proxy1).expect("proxy1 should exist");
        assert_eq!(rel1.proxy_type, ProxyType::UUPS);
        assert_eq!(rel1.implementation, Some(impl1));

        let rel2 = relationships.get(&proxy2).expect("proxy2 should exist");
        assert_eq!(rel2.proxy_type, ProxyType::Beacon);
        assert_eq!(rel2.beacon, Some(beacon_addr));

        // Verify independence: they don't share state
        assert_ne!(rel1.proxy_address, rel2.proxy_address);
    }

    #[test]
    fn link_proxy_to_matching_deployment() {
        let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let impl_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");

        // Create deployment + proxy events
        let deployed = sample_deployed_event(proxy_addr, "my-proxy");
        let upgraded = Upgraded { implementation: impl_addr };

        let log1 = make_log(Address::ZERO, deployed.encode_log_data());
        let log2 = make_log(proxy_addr, upgraded.encode_log_data());
        let parsed = decode_events(&[log1, log2]);

        let deployments = extract_deployments(&parsed, None);
        let relationships = detect_proxy_relationships(&parsed);

        let rel = relationships.get(&proxy_addr).expect("proxy should exist");
        let idx = link_proxy_to_deployment(rel, &deployments);
        assert_eq!(idx, Some(0));
        assert_eq!(deployments[idx.unwrap()].address, proxy_addr);
    }

    #[test]
    fn link_proxy_no_matching_deployment() {
        let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let other_addr = address!("5FbDB2315678afecb367f032d93F642f64180aa3");

        let rel = ProxyRelationship {
            proxy_address: proxy_addr,
            proxy_type: ProxyType::UUPS,
            implementation: Some(address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0")),
            admin: None,
            beacon: None,
        };

        // Deployment at a different address
        let deployed = sample_deployed_event(other_addr, "other-contract");
        let log = make_log(Address::ZERO, deployed.encode_log_data());
        let parsed = decode_events(&[log]);
        let deployments = extract_deployments(&parsed, None);

        let idx = link_proxy_to_deployment(&rel, &deployments);
        assert!(idx.is_none());
    }

    #[test]
    fn no_proxy_events_returns_empty_map() {
        let deployed = sample_deployed_event(
            address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
            "plain-contract",
        );
        let log = make_log(Address::ZERO, deployed.encode_log_data());
        let parsed = decode_events(&[log]);

        let relationships = detect_proxy_relationships(&parsed);
        assert!(relationships.is_empty());
    }

    #[test]
    fn admin_changed_before_upgraded_still_transparent() {
        let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let impl_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
        let admin_addr = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");

        // AdminChanged arrives before Upgraded (order shouldn't matter)
        let admin_changed = AdminChanged { previousAdmin: Address::ZERO, newAdmin: admin_addr };
        let upgraded = Upgraded { implementation: impl_addr };

        let log1 = make_log(proxy_addr, admin_changed.encode_log_data());
        let log2 = make_log(proxy_addr, upgraded.encode_log_data());
        let parsed = decode_events(&[log1, log2]);

        let relationships = detect_proxy_relationships(&parsed);
        assert_eq!(relationships.len(), 1);

        let rel = relationships.get(&proxy_addr).expect("proxy should exist");
        assert_eq!(rel.proxy_type, ProxyType::Transparent);
        assert_eq!(rel.implementation, Some(impl_addr));
        assert_eq!(rel.admin, Some(admin_addr));
    }
}
