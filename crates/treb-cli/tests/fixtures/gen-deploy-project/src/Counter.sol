// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Counter {
    uint256 public number;
    address public owner;

    constructor(uint256 initialCount, address _owner) {
        number = initialCount;
        owner = _owner;
    }

    function initialize() public {}

    function increment() public {
        number++;
    }
}
