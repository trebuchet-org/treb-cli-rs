// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

contract UpgradeableCounter {
    address private _owner;
    uint256 private _count;
    bool private _initialized;

    modifier onlyOwner() {
        require(msg.sender == _owner, "not owner");
        _;
    }

    function initialize() external {
        require(!_initialized, "already initialized");
        _owner = msg.sender;
        _initialized = true;
    }

    function increment() external {
        _count++;
    }

    function getCount() external view returns (uint256) {
        return _count;
    }

    function owner() external view returns (address) {
        return _owner;
    }

    function transferOwnership(address newOwner) external onlyOwner {
        require(newOwner != address(0), "zero address");
        _owner = newOwner;
    }
}
