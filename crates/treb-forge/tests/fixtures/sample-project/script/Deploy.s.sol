// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

import {Counter} from "../src/Counter.sol";

contract DeployScript {
    function run() public {
        Counter counter = new Counter();
        counter.setNumber(42);
    }
}
