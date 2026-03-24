// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

import "forge-std/Script.sol";
import "../src/Counter.sol";

struct DeploymentDetails {
    string artifact;
    string label;
    string entropy;
    bytes32 salt;
    bytes32 bytecodeHash;
    bytes32 initCodeHash;
    bytes constructorArgs;
    string createStrategy;
}

struct TxDetails {
    address to;
    bytes data;
    uint256 value;
}

struct SimTx {
    bytes32 transactionId;
    string senderId;
    address sender;
    bytes returnData;
    TxDetails transaction;
}

/// @title Deploy through Governor with proposal title and description.
/// @dev Emits GovernorBroadcast before the transaction to test that treb
///      captures the proposal metadata from script logs.
contract DeployViaGovernorWithDescriptionScript is Script {
    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );

    event TransactionSimulated(SimTx[] transactions);

    /// @dev Same event signature as ITrebEvents.GovernorBroadcast
    event GovernorBroadcast(address indexed governor, string title, string description);

    /// @dev Same event signature as ITrebEvents.GovernorProposalCreated
    event GovernorProposalCreated(
        uint256 indexed proposalId,
        address indexed governor,
        address indexed proposer,
        bytes32[] transactionIds
    );

    function run() public {
        address timelockAddress = vm.envAddress("TIMELOCK_ADDRESS");
        address governorAddress = vm.envAddress("GOVERNOR_ADDRESS");

        // Emit GovernorBroadcast with title and description BEFORE broadcasting
        emit GovernorBroadcast(
            governorAddress,
            "Deploy Counter v2",
            "This proposal deploys a new Counter contract via governance."
        );

        vm.startBroadcast(timelockAddress);

        Counter counter = new Counter();

        bytes32 txId = keccak256(
            abi.encode(block.chainid, block.number, address(counter))
        );
        bytes32 initCodeHash = keccak256(type(Counter).creationCode);
        bytes32 bytecodeHash = keccak256(address(counter).code);

        emit ContractDeployed(
            msg.sender,
            address(counter),
            txId,
            DeploymentDetails({
                artifact: "Counter",
                label: "Counter",
                entropy: "",
                salt: bytes32(0),
                bytecodeHash: bytecodeHash,
                initCodeHash: initCodeHash,
                constructorArgs: bytes(""),
                createStrategy: "create"
            })
        );

        SimTx[] memory txs = new SimTx[](1);
        txs[0] = SimTx({
            transactionId: txId,
            senderId: "governance",
            sender: msg.sender,
            returnData: bytes(""),
            transaction: TxDetails({to: address(0), data: bytes(""), value: 0})
        });
        emit TransactionSimulated(txs);

        // Emit GovernorProposalCreated to link the description to a proposal
        bytes32[] memory transactionIds = new bytes32[](1);
        transactionIds[0] = txId;
        emit GovernorProposalCreated(
            uint256(keccak256(abi.encode(governorAddress, txId))),
            governorAddress,
            msg.sender,
            transactionIds
        );

        vm.stopBroadcast();
    }
}
