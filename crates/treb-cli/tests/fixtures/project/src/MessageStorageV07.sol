// SPDX-License-Identifier: MIT
pragma solidity =0.7.6;

import {StringUtils} from "./StringUtils.sol";

contract MessageStorageV07 {
    string public message;

    constructor(string memory _message) {
        message = _message;
    }

    function setMessage(string calldata _message) external {
        message = _message;
    }

    function getMessageLength() external view returns (uint256) {
        return StringUtils.length(message);
    }

    function getUpperMessage() external view returns (string memory) {
        return StringUtils.toUpperCase(message);
    }
}
