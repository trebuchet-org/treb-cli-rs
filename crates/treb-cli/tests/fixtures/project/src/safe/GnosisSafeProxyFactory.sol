// SPDX-License-Identifier: LGPL-3.0-only
pragma solidity =0.8.30;

import "./GnosisSafeProxy.sol";

/// @title Minimal SafeProxyFactory stub for treb integration tests.
/// @dev Deploys GnosisSafeProxy instances via CREATE2 and optionally calls an
///      initializer on the new proxy — matching the Safe v1.4.1 factory interface.
contract GnosisSafeProxyFactory {
    event ProxyCreation(GnosisSafeProxy indexed proxy, address singleton);

    /// @notice Deploy a proxy with CREATE2, call initializer, emit event.
    function createProxyWithNonce(
        address _singleton,
        bytes memory initializer,
        uint256 saltNonce
    ) public returns (GnosisSafeProxy proxy) {
        bytes32 salt = keccak256(abi.encodePacked(keccak256(initializer), saltNonce));
        proxy = _deployProxy(_singleton, initializer, salt);
        emit ProxyCreation(proxy, _singleton);
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    function _deployProxy(
        address _singleton,
        bytes memory initializer,
        bytes32 salt
    ) internal returns (GnosisSafeProxy proxy) {
        require(_isContract(_singleton), "Singleton contract not deployed");

        bytes memory deploymentData = abi.encodePacked(
            type(GnosisSafeProxy).creationCode,
            uint256(uint160(_singleton))
        );

        assembly {
            proxy := create2(0x0, add(0x20, deploymentData), mload(deploymentData), salt)
        }
        require(address(proxy) != address(0), "Create2 call failed");

        if (initializer.length > 0) {
            assembly {
                if eq(call(gas(), proxy, 0, add(initializer, 0x20), mload(initializer), 0, 0), 0) {
                    revert(0, 0)
                }
            }
        }
    }

    function _isContract(address account) internal view returns (bool) {
        uint256 size;
        assembly {
            size := extcodesize(account)
        }
        return size > 0;
    }
}
