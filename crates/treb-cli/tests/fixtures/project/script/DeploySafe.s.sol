// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

import "forge-std/Script.sol";
import "../src/safe/GnosisSafe.sol";
import "../src/safe/GnosisSafeProxy.sol";
import "../src/safe/GnosisSafeProxyFactory.sol";
import "../src/safe/MultiSend.sol";

/// @title Deploy Safe infrastructure on Anvil for integration tests.
/// @dev Deploys singleton, factory, multisend, and creates a proxy with
///      configurable owners/threshold via environment variables.
///
/// Environment variables:
///   SAFE_OWNERS      — comma-separated owner addresses (required)
///   SAFE_THRESHOLD   — signature threshold (required)
///   SAFE_SALT_NONCE  — CREATE2 salt nonce (optional, defaults to 0)
contract DeploySafeScript is Script {
    event SafeInfraDeployed(
        address singleton,
        address factory,
        address multisend,
        address proxy,
        address[] owners,
        uint256 threshold
    );

    function run() public {
        address[] memory owners = vm.envAddress("SAFE_OWNERS", ",");
        uint256 safeThreshold = vm.envUint("SAFE_THRESHOLD");
        uint256 saltNonce = vm.envOr("SAFE_SALT_NONCE", uint256(0));

        vm.startBroadcast();

        // Deploy core Safe infrastructure
        GnosisSafe singleton = new GnosisSafe();
        GnosisSafeProxyFactory factory = new GnosisSafeProxyFactory();
        MultiSend multisend = new MultiSend();

        // Create Safe proxy with configured owners and threshold
        bytes memory initializer = abi.encodeWithSelector(
            GnosisSafe.setup.selector,
            owners,
            safeThreshold,
            address(0),
            bytes(""),
            address(0),
            address(0),
            0,
            payable(address(0))
        );

        GnosisSafeProxy proxy = factory.createProxyWithNonce(
            address(singleton),
            initializer,
            saltNonce
        );

        emit SafeInfraDeployed(
            address(singleton),
            address(factory),
            address(multisend),
            address(proxy),
            owners,
            safeThreshold
        );

        vm.stopBroadcast();
    }
}
