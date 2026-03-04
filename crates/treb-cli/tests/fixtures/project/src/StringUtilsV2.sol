// SPDX-License-Identifier: MIT
pragma solidity ^0.7.0 || ^0.8.0;

library StringUtilsV2 {
    function concat(string calldata a, string calldata b) external pure returns (string memory) {
        return string(abi.encodePacked(a, b));
    }

    function length(string calldata s) external pure returns (uint256) {
        return bytes(s).length;
    }

    function toUpperCase(string calldata s) external pure returns (string memory) {
        bytes memory b = bytes(s);
        for (uint256 i = 0; i < b.length; i++) {
            if (b[i] >= 0x61 && b[i] <= 0x7A) {
                b[i] = bytes1(uint8(b[i]) - 32);
            }
        }
        return string(b);
    }

    function equal(string calldata a, string calldata b) external pure returns (bool) {
        return keccak256(abi.encodePacked(a)) == keccak256(abi.encodePacked(b));
    }
}
