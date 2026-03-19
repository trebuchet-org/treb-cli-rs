// SPDX-License-Identifier: LGPL-3.0-only
pragma solidity =0.8.30;

/// @title Minimal SafeProxy stub for treb integration tests.
/// @dev Delegates all calls to the singleton (stored at slot 0) via DELEGATECALL.
///      Matches the Safe v1.4.1 proxy layout exactly.
contract GnosisSafeProxy {
    /// @dev Slot 0: singleton implementation address.
    address internal singleton;

    constructor(address _singleton) {
        require(_singleton != address(0), "Invalid singleton address provided");
        singleton = _singleton;
    }

    /// @dev Forwards all calls to singleton via DELEGATECALL.
    fallback() external payable {
        assembly {
            let _singleton := and(sload(0), 0xffffffffffffffffffffffffffffffffffffffff)
            calldatacopy(0, 0, calldatasize())
            let success := delegatecall(gas(), _singleton, 0, calldatasize(), 0, 0)
            returndatacopy(0, 0, returndatasize())
            if eq(success, 0) {
                revert(0, returndatasize())
            }
            return(0, returndatasize())
        }
    }

    receive() external payable {}
}
