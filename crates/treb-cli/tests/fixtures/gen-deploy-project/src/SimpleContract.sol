// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract SimpleContract {
    uint256 public value;

    function initialize() public {}

    function setValue(uint256 newValue) public {
        value = newValue;
    }
}
