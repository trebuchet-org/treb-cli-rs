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

/// @title Deploy contracts through both a wallet sender and a Governor sender.
/// @dev Uses two separate vm.startBroadcast()/vm.stopBroadcast() blocks:
///      1. Default sender (wallet) deploys WalletCounter
///      2. Timelock address deploys GovernorCounter
///      This exercises partition_into_runs() with mixed sender types.
///
/// Environment variables:
///   TIMELOCK_ADDRESS — the timelock proxy address to broadcast from (required)
contract DeployMixedWalletGovernorScript is Script {
    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );

    event TransactionSimulated(SimTx[] transactions);

    function run() public {
        address timelockAddress = vm.envAddress("TIMELOCK_ADDRESS");

        // --- Wallet sender broadcast ---
        vm.startBroadcast();

        Counter walletCounter = new Counter();

        bytes32 walletTxId = keccak256(
            abi.encode(block.chainid, block.number, address(walletCounter))
        );
        bytes32 walletInitCodeHash = keccak256(type(Counter).creationCode);
        bytes32 walletBytecodeHash = keccak256(address(walletCounter).code);

        emit ContractDeployed(
            msg.sender,
            address(walletCounter),
            walletTxId,
            DeploymentDetails({
                artifact: "Counter",
                label: "WalletCounter",
                entropy: "",
                salt: bytes32(0),
                bytecodeHash: walletBytecodeHash,
                initCodeHash: walletInitCodeHash,
                constructorArgs: bytes(""),
                createStrategy: "create"
            })
        );

        SimTx[] memory walletTxs = new SimTx[](1);
        walletTxs[0] = SimTx({
            transactionId: walletTxId,
            senderId: "anvil",
            sender: msg.sender,
            returnData: bytes(""),
            transaction: TxDetails({to: address(0), data: bytes(""), value: 0})
        });
        emit TransactionSimulated(walletTxs);

        vm.stopBroadcast();

        // --- Governor sender broadcast (via timelock) ---
        vm.startBroadcast(timelockAddress);

        Counter governorCounter = new Counter();

        bytes32 govTxId = keccak256(
            abi.encode(block.chainid, block.number, address(governorCounter))
        );
        bytes32 govInitCodeHash = keccak256(type(Counter).creationCode);
        bytes32 govBytecodeHash = keccak256(address(governorCounter).code);

        emit ContractDeployed(
            msg.sender,
            address(governorCounter),
            govTxId,
            DeploymentDetails({
                artifact: "Counter",
                label: "GovernorCounter",
                entropy: "",
                salt: bytes32(0),
                bytecodeHash: govBytecodeHash,
                initCodeHash: govInitCodeHash,
                constructorArgs: bytes(""),
                createStrategy: "create"
            })
        );

        SimTx[] memory govTxs = new SimTx[](1);
        govTxs[0] = SimTx({
            transactionId: govTxId,
            senderId: "governance",
            sender: msg.sender,
            returnData: bytes(""),
            transaction: TxDetails({to: address(0), data: bytes(""), value: 0})
        });
        emit TransactionSimulated(govTxs);

        vm.stopBroadcast();
    }
}
