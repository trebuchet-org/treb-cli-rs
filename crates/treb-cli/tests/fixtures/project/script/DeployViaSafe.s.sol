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

/// @title Deploy a contract through a Safe address.
/// @dev Uses vm.startBroadcast(safeAddress) so all transactions appear to
///      originate from the Safe, triggering treb's Safe routing pipeline.
///
/// Environment variables:
///   SAFE_ADDRESS — the Safe proxy address to broadcast from (required)
contract DeployViaSafeScript is Script {
    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );

    event TransactionSimulated(SimTx[] transactions);

    function run() public {
        address safeAddress = vm.envAddress("SAFE_ADDRESS");

        vm.startBroadcast(safeAddress);

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
            senderId: "safe",
            sender: msg.sender,
            returnData: bytes(""),
            transaction: TxDetails({to: address(0), data: bytes(""), value: 0})
        });
        emit TransactionSimulated(txs);

        vm.stopBroadcast();
    }
}
