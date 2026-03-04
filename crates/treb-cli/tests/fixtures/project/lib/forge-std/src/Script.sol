// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

interface Vm {
    function startBroadcast() external;
    function stopBroadcast() external;
}

library console {
    function log(string memory, address) internal pure {}
}

abstract contract Script {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
}
