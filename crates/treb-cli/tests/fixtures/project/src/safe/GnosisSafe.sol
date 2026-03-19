// SPDX-License-Identifier: LGPL-3.0-only
pragma solidity =0.8.30;

/// @title Minimal GnosisSafe stub for treb integration tests.
/// @dev Implements only the subset of Safe v1.4.1 needed by fork_routing.rs:
///      setup, execTransaction with pre-approved hash signatures (v=1),
///      approveHash, getOwners, getThreshold, nonce, and EIP-712 tx hashing.
///      Storage slot 0 is reserved for the proxy's singleton pointer.
contract GnosisSafe {
    // -----------------------------------------------------------------------
    // Storage — slot 0 must be singleton for proxy compatibility
    // -----------------------------------------------------------------------

    /// @dev Slot 0: singleton address stored by SafeProxy. Never written by Safe code.
    address internal _singleton;

    /// @dev Linked-list sentinel for owner management (matches Safe v1.4.1).
    address internal constant SENTINEL_OWNERS = address(0x1);

    mapping(address => address) internal owners;
    uint256 internal ownerCount;
    uint256 public threshold;
    uint256 public nonce;
    mapping(address => mapping(bytes32 => uint256)) public approvedHashes;

    // -----------------------------------------------------------------------
    // EIP-712 constants (identical to Safe v1.4.1)
    // -----------------------------------------------------------------------

    bytes32 private constant DOMAIN_SEPARATOR_TYPEHASH =
        0x47e79534a245952e8b16893a336b85a3d9ea9fa8c573f3d803afb92a79469218;

    bytes32 private constant SAFE_TX_TYPEHASH =
        0xbb8310d486368db6bd6f849402fdd73ad53d316b5a4b2644ad6efe0f941286d8;

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    event SafeSetup(address indexed initiator, address[] owners, uint256 threshold);
    event ExecutionSuccess(bytes32 indexed txHash, uint256 payment);
    event ExecutionFailure(bytes32 indexed txHash, uint256 payment);
    event ApproveHash(bytes32 indexed approvedHash, address indexed owner);

    // -----------------------------------------------------------------------
    // Constructor — makes the singleton contract unusable directly
    // -----------------------------------------------------------------------

    constructor() {
        threshold = 1;
    }

    // -----------------------------------------------------------------------
    // Setup
    // -----------------------------------------------------------------------

    /// @notice Initialise Safe owners and threshold. Can only be called once
    ///         (threshold must be 0, which is the default for a fresh proxy).
    function setup(
        address[] calldata _owners,
        uint256 _threshold,
        address,          // to            (unused in stub)
        bytes calldata,   // data          (unused in stub)
        address,          // fallbackHandler (unused in stub)
        address,          // paymentToken  (unused in stub)
        uint256,          // payment       (unused in stub)
        address payable   // paymentReceiver (unused in stub)
    ) external {
        require(threshold == 0, "GS200");
        require(_threshold <= _owners.length, "GS201");
        require(_threshold >= 1, "GS202");

        address currentOwner = SENTINEL_OWNERS;
        for (uint256 i = 0; i < _owners.length; i++) {
            address owner = _owners[i];
            require(
                owner != address(0) && owner != SENTINEL_OWNERS && owner != address(this) && currentOwner != owner,
                "GS203"
            );
            require(owners[owner] == address(0), "GS204");
            owners[currentOwner] = owner;
            currentOwner = owner;
        }
        owners[currentOwner] = SENTINEL_OWNERS;
        ownerCount = _owners.length;
        threshold = _threshold;

        emit SafeSetup(msg.sender, _owners, _threshold);
    }

    // -----------------------------------------------------------------------
    // execTransaction
    // -----------------------------------------------------------------------

    /// @notice Execute a Safe transaction after validating pre-approved signatures.
    function execTransaction(
        address to,
        uint256 value,
        bytes calldata data,
        uint8 operation,
        uint256 safeTxGas,
        uint256 baseGas,
        uint256 gasPrice,
        address gasToken,
        address payable refundReceiver,
        bytes memory signatures
    ) public payable returns (bool success) {
        bytes32 txHash;
        {
            bytes memory txHashData = encodeTransactionData(
                to, value, data, operation,
                safeTxGas, baseGas, gasPrice, gasToken, refundReceiver,
                nonce
            );
            nonce++;
            txHash = keccak256(txHashData);
            _checkSignatures(txHash, signatures);
        }

        // Execute
        bytes memory _data = data; // calldata → memory for low-level call
        if (operation == 1) {
            (success,) = to.delegatecall(_data);
        } else {
            (success,) = to.call{value: value}(_data);
        }

        require(success || safeTxGas != 0 || gasPrice != 0, "GS013");

        if (success) emit ExecutionSuccess(txHash, 0);
        else emit ExecutionFailure(txHash, 0);
    }

    // -----------------------------------------------------------------------
    // Signature verification (v=1 pre-approved hashes only)
    // -----------------------------------------------------------------------

    function _checkSignatures(bytes32 dataHash, bytes memory signatures) internal view {
        uint256 _threshold = threshold;
        require(_threshold > 0, "GS001");
        require(signatures.length >= _threshold * 65, "GS020");

        address lastOwner = address(0);
        for (uint256 i = 0; i < _threshold; i++) {
            (uint8 v, bytes32 r,) = _signatureSplit(signatures, i);

            address currentOwner;
            if (v == 1) {
                // Pre-approved hash: r = left-padded owner address, s = 0
                currentOwner = address(uint160(uint256(r)));
                require(
                    msg.sender == currentOwner || approvedHashes[currentOwner][dataHash] != 0,
                    "GS025"
                );
            } else {
                revert("Only pre-approved signatures (v=1) supported in test stub");
            }

            // Ascending order, no duplicates, must be an owner
            require(
                currentOwner > lastOwner && owners[currentOwner] != address(0) && currentOwner != SENTINEL_OWNERS,
                "GS026"
            );
            lastOwner = currentOwner;
        }
    }

    // -----------------------------------------------------------------------
    // approveHash
    // -----------------------------------------------------------------------

    /// @notice Mark a hash as approved by msg.sender (must be an owner).
    function approveHash(bytes32 hashToApprove) external {
        require(owners[msg.sender] != address(0), "GS030");
        approvedHashes[msg.sender][hashToApprove] = 1;
        emit ApproveHash(hashToApprove, msg.sender);
    }

    // -----------------------------------------------------------------------
    // View helpers
    // -----------------------------------------------------------------------

    function getOwners() public view returns (address[] memory) {
        address[] memory result = new address[](ownerCount);
        uint256 index = 0;
        address currentOwner = owners[SENTINEL_OWNERS];
        while (currentOwner != SENTINEL_OWNERS) {
            result[index] = currentOwner;
            currentOwner = owners[currentOwner];
            index++;
        }
        return result;
    }

    function getThreshold() public view returns (uint256) {
        return threshold;
    }

    // -----------------------------------------------------------------------
    // EIP-712
    // -----------------------------------------------------------------------

    function domainSeparator() public view returns (bytes32) {
        return keccak256(abi.encode(DOMAIN_SEPARATOR_TYPEHASH, block.chainid, this));
    }

    function encodeTransactionData(
        address to,
        uint256 value,
        bytes calldata data,
        uint8 operation,
        uint256 safeTxGas,
        uint256 _baseGas,
        uint256 _gasPrice,
        address gasToken,
        address refundReceiver,
        uint256 _nonce
    ) public view returns (bytes memory) {
        bytes32 safeTxHash = keccak256(
            abi.encode(
                SAFE_TX_TYPEHASH,
                to,
                value,
                keccak256(data),
                operation,
                safeTxGas,
                _baseGas,
                _gasPrice,
                gasToken,
                refundReceiver,
                _nonce
            )
        );
        return abi.encodePacked(bytes1(0x19), bytes1(0x01), domainSeparator(), safeTxHash);
    }

    function getTransactionHash(
        address to,
        uint256 value,
        bytes calldata data,
        uint8 operation,
        uint256 safeTxGas,
        uint256 _baseGas,
        uint256 _gasPrice,
        address gasToken,
        address refundReceiver,
        uint256 _nonce
    ) public view returns (bytes32) {
        return keccak256(
            encodeTransactionData(
                to, value, data, operation,
                safeTxGas, _baseGas, _gasPrice, gasToken, refundReceiver,
                _nonce
            )
        );
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// @dev Split packed signature at position `pos` into (v, r, s).
    function _signatureSplit(bytes memory signatures, uint256 pos)
        internal
        pure
        returns (uint8 v, bytes32 r, bytes32 s)
    {
        assembly {
            let sigPos := mul(0x41, pos)
            r := mload(add(signatures, add(sigPos, 0x20)))
            s := mload(add(signatures, add(sigPos, 0x40)))
            v := byte(0, mload(add(signatures, add(sigPos, 0x60))))
        }
    }
}
