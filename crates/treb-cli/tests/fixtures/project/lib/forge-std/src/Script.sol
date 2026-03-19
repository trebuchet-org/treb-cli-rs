// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

interface Vm {
    function startBroadcast() external;
    function startBroadcast(address signer) external;
    function stopBroadcast() external;
    function envAddress(string calldata name) external view returns (address);
    function envAddress(string calldata name, string calldata delim) external view returns (address[] memory);
    function envUint(string calldata name) external view returns (uint256);
    function envOr(string calldata name, uint256 defaultValue) external view returns (uint256);
}

library console {
    function log(string memory, address) internal pure {}
}

abstract contract Script {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
}
