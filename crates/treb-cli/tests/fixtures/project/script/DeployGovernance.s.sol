// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

import "forge-std/Script.sol";
import "../src/governance/GovernanceToken.sol";
import "../src/governance/TrebTimelock.sol";
import "../src/governance/TrebGovernor.sol";

/// @title Deploy governance infrastructure on Anvil for integration tests.
/// @dev Deploys GovernanceToken, TrebTimelock, TrebGovernor with correct
///      access control (governor gets PROPOSER_ROLE on timelock).
///
/// Environment variables:
///   GOV_MIN_DELAY     — timelock minimum delay in seconds (optional, defaults to 1)
///   GOV_VOTING_DELAY  — governor voting delay in blocks (optional, defaults to 1)
///   GOV_VOTING_PERIOD — governor voting period in blocks (optional, defaults to 50)
///   GOV_QUORUM        — quorum threshold in votes (optional, defaults to 1)
contract DeployGovernanceScript is Script {
    event GovernanceDeployed(
        address token,
        address timelock,
        address governor,
        uint256 minDelay,
        uint256 votingDelay,
        uint256 votingPeriod,
        uint256 quorumThreshold
    );

    function run() public {
        uint256 minDelay = vm.envOr("GOV_MIN_DELAY", uint256(1));
        uint256 votingDelay = vm.envOr("GOV_VOTING_DELAY", uint256(1));
        uint256 votingPeriod = vm.envOr("GOV_VOTING_PERIOD", uint256(50));
        uint256 quorumThreshold = vm.envOr("GOV_QUORUM", uint256(1));

        vm.startBroadcast();

        // 1. Deploy governance token
        GovernanceToken token = new GovernanceToken("Governance Token", "GOV");

        // 2. Deploy timelock with empty proposers/executors — we configure
        //    roles after the governor address is known.
        address[] memory empty = new address[](0);
        TrebTimelock timelock = new TrebTimelock(minDelay, empty, empty, msg.sender);

        // 3. Deploy governor (needs token + timelock addresses)
        TrebGovernor governor = new TrebGovernor(
            address(token),
            address(timelock),
            votingDelay,
            votingPeriod,
            quorumThreshold
        );

        // 4. Grant PROPOSER_ROLE to governor on timelock
        timelock.grantRole(timelock.PROPOSER_ROLE(), address(governor));

        emit GovernanceDeployed(
            address(token),
            address(timelock),
            address(governor),
            minDelay,
            votingDelay,
            votingPeriod,
            quorumThreshold
        );

        vm.stopBroadcast();
    }
}
