// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

import "forge-std/Script.sol";

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

contract GovernorProposalScript is Script {
    event TransactionSimulated(SimTx[] transactions);

    event GovernorProposalCreated(
        uint256 indexed proposalId,
        address indexed governor,
        address indexed proposer,
        bytes32[] transactionIds
    );

    function run() public {
        vm.startBroadcast();

        address governor = 0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa;
        address proposer = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
        bytes32 txId = keccak256("governor-proposal-tx-1");

        SimTx[] memory txs = new SimTx[](1);
        txs[0] = SimTx({
            transactionId: txId,
            senderId: "governance",
            sender: governor,
            returnData: bytes(""),
            transaction: TxDetails({to: address(0x1000), data: hex"deadbeef", value: 0})
        });
        emit TransactionSimulated(txs);

        bytes32[] memory transactionIds = new bytes32[](1);
        transactionIds[0] = txId;
        emit GovernorProposalCreated(42, governor, proposer, transactionIds);

        vm.stopBroadcast();
    }
}
