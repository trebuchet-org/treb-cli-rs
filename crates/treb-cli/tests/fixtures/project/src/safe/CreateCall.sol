// SPDX-License-Identifier: LGPL-3.0-only
pragma solidity =0.8.30;

/// @title Minimal CreateCall for Safe DelegateCall-based contract deployment.
/// @dev The Safe DelegateCalls to this contract to execute CREATE on behalf
///      of the Safe. When DelegateCalled, `create` runs in the Safe's context
///      making the Safe the deployer (`msg.sender` for the new contract).
contract CreateCall {
    event ContractCreation(address indexed newContract);

    /// @notice Deploy a contract using CREATE via DelegateCall from a Safe.
    /// @param value ETH value to pass to the new contract's constructor.
    /// @param deploymentData The full contract creation bytecode.
    /// @return newContract The address of the newly deployed contract.
    function performCreate(
        uint256 value,
        bytes memory deploymentData
    ) public returns (address newContract) {
        // solhint-disable-next-line no-inline-assembly
        assembly {
            newContract := create(value, add(deploymentData, 0x20), mload(deploymentData))
        }
        require(newContract != address(0), "Could not deploy contract");
        emit ContractCreation(newContract);
    }
}
