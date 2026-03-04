// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

import {StringUtilsV2} from "./StringUtilsV2.sol";

contract MessageStorageV08 {
    string public message;

    constructor(string memory _message) {
        message = _message;
    }

    function setMessage(string calldata _message) external {
        message = _message;
    }

    function getMessageLength() external view returns (uint256) {
        return StringUtilsV2.length(message);
    }

    function getUpperMessage() external view returns (string memory) {
        return StringUtilsV2.toUpperCase(message);
    }
}
