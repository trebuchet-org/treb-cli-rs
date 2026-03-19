// SPDX-License-Identifier: LGPL-3.0-only
pragma solidity =0.8.30;

/// @title Minimal MultiSend stub for treb integration tests.
/// @dev Unpacks the packed encoding format used by treb_safe::encode_multi_send():
///      operation (1 byte) + to (20 bytes) + value (32 bytes) + dataLen (32 bytes) + data (N bytes).
///      Supports CALL (0) and DELEGATECALL (1). Assembly matches Safe v1.4.1 MultiSend.
contract MultiSend {
    /// @dev Sends multiple transactions; reverts all if one fails.
    /// @param transactions Packed encoded transactions (see format above).
    function multiSend(bytes memory transactions) public payable {
        assembly {
            let length := mload(transactions)
            let i := 0x20
            for {} lt(i, add(length, 0x20)) {} {
                let operation := shr(0xf8, mload(add(transactions, i)))
                let to := shr(0x60, mload(add(transactions, add(i, 0x01))))
                let value := mload(add(transactions, add(i, 0x15)))
                let dataLength := mload(add(transactions, add(i, 0x35)))
                let data := add(transactions, add(i, 0x55))
                let success := 0
                switch operation
                case 0 { success := call(gas(), to, value, data, dataLength, 0, 0) }
                case 1 { success := delegatecall(gas(), to, data, dataLength, 0, 0) }
                if eq(success, 0) { revert(0, 0) }
                i := add(i, add(0x55, dataLength))
            }
        }
    }
}
