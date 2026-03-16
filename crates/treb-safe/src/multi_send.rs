//! MultiSend encoding for batching Safe transactions.
//!
//! Implements the Gnosis Safe MultiSend contract's packed encoding format
//! for batching multiple operations into a single Safe transaction.

use alloy_primitives::{Address, Bytes, U256};

/// A single operation in a MultiSend batch.
#[derive(Debug, Clone)]
pub struct MultiSendOperation {
    /// 0 = Call, 1 = DelegateCall.
    pub operation: u8,
    /// Target contract address.
    pub to: Address,
    /// ETH value to send.
    pub value: U256,
    /// Calldata.
    pub data: Bytes,
}

/// Well-known MultiSend contract address (deployed across most chains).
pub const MULTI_SEND_ADDRESS: Address = {
    // 0x38869bf66a61cF6bDB996A6aE40D5853Fd43B526 (MultiSend v1.4.1)
    let bytes: [u8; 20] = [
        0x38, 0x86, 0x9b, 0xf6, 0x6a, 0x61, 0xcf, 0x6b, 0xdb, 0x99,
        0x6a, 0x6a, 0xe4, 0x0d, 0x58, 0x53, 0xfd, 0x43, 0xb5, 0x26,
    ];
    Address::new(bytes)
};

/// Encode operations into MultiSend's packed `transactions` bytes.
///
/// Each operation is packed as:
/// ```text
/// operation (uint8, 1 byte)
/// to        (address, 20 bytes)
/// value     (uint256, 32 bytes)
/// dataLen   (uint256, 32 bytes)
/// data      (bytes, variable)
/// ```
///
/// The result can be passed to `MultiSend.multiSend(bytes)`.
pub fn encode_multi_send(operations: &[MultiSendOperation]) -> Bytes {
    let mut packed = Vec::new();

    for op in operations {
        // operation: 1 byte
        packed.push(op.operation);
        // to: 20 bytes
        packed.extend_from_slice(op.to.as_slice());
        // value: 32 bytes (big-endian)
        packed.extend_from_slice(&op.value.to_be_bytes::<32>());
        // dataLen: 32 bytes (big-endian)
        let data_len = U256::from(op.data.len());
        packed.extend_from_slice(&data_len.to_be_bytes::<32>());
        // data: variable
        packed.extend_from_slice(&op.data);
    }

    Bytes::from(packed)
}

/// Build the full `multiSend(bytes)` calldata for a Safe DelegateCall.
///
/// Returns the ABI-encoded function call to `MultiSend.multiSend(bytes memory transactions)`.
/// The selector is `0x8d80ff0a`.
pub fn encode_multi_send_call(operations: &[MultiSendOperation]) -> Bytes {
    let packed = encode_multi_send(operations);
    // multiSend(bytes) selector = 0x8d80ff0a
    let mut calldata = vec![0x8d, 0x80, 0xff, 0x0a];
    // ABI-encode the packed bytes as a single `bytes` parameter:
    // offset (32 bytes) + length (32 bytes) + data (padded to 32-byte boundary)
    let offset = U256::from(32u64);
    calldata.extend_from_slice(&offset.to_be_bytes::<32>());
    let length = U256::from(packed.len());
    calldata.extend_from_slice(&length.to_be_bytes::<32>());
    calldata.extend_from_slice(&packed);
    // Pad to 32-byte boundary
    let padding = (32 - (packed.len() % 32)) % 32;
    calldata.extend(std::iter::repeat_n(0u8, padding));
    Bytes::from(calldata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn encode_single_operation() {
        let ops = vec![MultiSendOperation {
            operation: 0,
            to: address!("0000000000000000000000000000000000000042"),
            value: U256::ZERO,
            data: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]),
        }];

        let packed = encode_multi_send(&ops);
        // 1 + 20 + 32 + 32 + 4 = 89 bytes
        assert_eq!(packed.len(), 89);
        // First byte is operation (0 = Call)
        assert_eq!(packed[0], 0);
        // Bytes 1..21 are the address
        assert_eq!(&packed[1..21], address!("0000000000000000000000000000000000000042").as_slice());
    }

    #[test]
    fn encode_multiple_operations() {
        let ops = vec![
            MultiSendOperation {
                operation: 0,
                to: address!("0000000000000000000000000000000000000001"),
                value: U256::ZERO,
                data: Bytes::from(vec![0x01]),
            },
            MultiSendOperation {
                operation: 0,
                to: address!("0000000000000000000000000000000000000002"),
                value: U256::from(100u64),
                data: Bytes::new(),
            },
        ];

        let packed = encode_multi_send(&ops);
        // First op: 1 + 20 + 32 + 32 + 1 = 86
        // Second op: 1 + 20 + 32 + 32 + 0 = 85
        assert_eq!(packed.len(), 86 + 85);
    }

    #[test]
    fn encode_empty_operations() {
        let packed = encode_multi_send(&[]);
        assert!(packed.is_empty());
    }

    #[test]
    fn multi_send_call_has_correct_selector() {
        let ops = vec![MultiSendOperation {
            operation: 0,
            to: address!("0000000000000000000000000000000000000001"),
            value: U256::ZERO,
            data: Bytes::new(),
        }];

        let calldata = encode_multi_send_call(&ops);
        // Selector is 0x8d80ff0a
        assert_eq!(&calldata[..4], &[0x8d, 0x80, 0xff, 0x0a]);
    }
}
