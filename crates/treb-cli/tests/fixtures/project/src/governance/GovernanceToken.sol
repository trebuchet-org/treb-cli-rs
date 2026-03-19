// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

/// @title Minimal ERC20Votes governance token stub for treb integration tests.
/// @dev Implements only the subset of OZ ERC20Votes needed by TrebGovernor:
///      mint, delegate, getVotes, getPastVotes, clock (ERC6372).
///      Checkpointing is simplified — getPastVotes always returns current votes.
contract GovernanceToken {
    string public name;
    string public symbol;
    uint8 public constant decimals = 18;
    uint256 public totalSupply;

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => address) public delegates;
    mapping(address => uint256) private _votingPower;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    event DelegateChanged(address indexed delegator, address indexed fromDelegate, address indexed toDelegate);

    constructor(string memory _name, string memory _symbol) {
        name = _name;
        symbol = _symbol;
    }

    function mint(address to, uint256 amount) external {
        totalSupply += amount;
        balanceOf[to] += amount;
        // Auto-update voting power if already delegated
        address d = delegates[to];
        if (d != address(0)) {
            _votingPower[d] += amount;
        }
        emit Transfer(address(0), to, amount);
    }

    function delegate(address delegatee) external {
        address oldDelegate = delegates[msg.sender];
        delegates[msg.sender] = delegatee;
        if (oldDelegate != address(0)) {
            _votingPower[oldDelegate] -= balanceOf[msg.sender];
        }
        if (delegatee != address(0)) {
            _votingPower[delegatee] += balanceOf[msg.sender];
        }
        emit DelegateChanged(msg.sender, oldDelegate, delegatee);
    }

    function getVotes(address account) external view returns (uint256) {
        return _votingPower[account];
    }

    /// @dev Simplified: returns current votes regardless of timepoint.
    function getPastVotes(address account, uint256) external view returns (uint256) {
        return _votingPower[account];
    }

    /// @dev ERC6372 — Governor reads this to know what clock to use.
    function clock() public view returns (uint48) {
        return uint48(block.number);
    }

    /// @dev ERC6372 clock mode descriptor.
    function CLOCK_MODE() public pure returns (string memory) {
        return "mode=blocknumber&from=default";
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
        return true;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
        return true;
    }
}
