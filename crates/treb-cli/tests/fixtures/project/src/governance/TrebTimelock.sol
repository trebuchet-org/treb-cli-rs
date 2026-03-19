// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

/// @title Minimal TimelockController stub for treb integration tests.
/// @dev Implements only the subset of OZ TimelockController needed by
///      fork_routing.rs: getMinDelay, scheduleBatch, executeBatch, grantRole.
///      Function signatures match the sol! ABI in fork_routing.rs exactly.
///      The timelock is its own admin (DEFAULT_ADMIN_ROLE granted to address(this)).
contract TrebTimelock {
    bytes32 public constant DEFAULT_ADMIN_ROLE = bytes32(0);
    bytes32 public constant PROPOSER_ROLE = keccak256("PROPOSER_ROLE");
    bytes32 public constant EXECUTOR_ROLE = keccak256("EXECUTOR_ROLE");
    bytes32 public constant CANCELLER_ROLE = keccak256("CANCELLER_ROLE");

    uint256 private _minDelay;

    /// @dev role => account => hasRole
    mapping(bytes32 => mapping(address => bool)) private _roles;

    /// @dev operationId => timestamp (0 = unset, 1 = done, >1 = ready at timestamp)
    mapping(bytes32 => uint256) private _timestamps;

    event CallScheduled(
        bytes32 indexed id,
        uint256 indexed index,
        address target,
        uint256 value,
        bytes data,
        bytes32 predecessor,
        uint256 delay
    );
    event CallExecuted(bytes32 indexed id, uint256 indexed index, address target, uint256 value, bytes data);
    event RoleGranted(bytes32 indexed role, address indexed account, address indexed sender);

    constructor(uint256 minDelay, address[] memory proposers, address[] memory executors, address admin) {
        _minDelay = minDelay;

        // Timelock is its own admin (matches OZ TimelockController behavior)
        _roles[DEFAULT_ADMIN_ROLE][address(this)] = true;

        // Grant admin role to deployer
        _roles[DEFAULT_ADMIN_ROLE][admin] = true;

        for (uint256 i = 0; i < proposers.length; i++) {
            _roles[PROPOSER_ROLE][proposers[i]] = true;
            _roles[CANCELLER_ROLE][proposers[i]] = true;
        }

        for (uint256 i = 0; i < executors.length; i++) {
            _roles[EXECUTOR_ROLE][executors[i]] = true;
        }
    }

    function getMinDelay() external view returns (uint256) {
        return _minDelay;
    }

    function hasRole(bytes32 role, address account) public view returns (bool) {
        return _roles[role][account];
    }

    function grantRole(bytes32 role, address account) external {
        require(
            _roles[DEFAULT_ADMIN_ROLE][msg.sender],
            "AccessControl: sender must be an admin to grant"
        );
        _roles[role][account] = true;
        emit RoleGranted(role, account, msg.sender);
    }

    function hashOperationBatch(
        address[] calldata targets,
        uint256[] calldata values,
        bytes[] calldata payloads,
        bytes32 predecessor,
        bytes32 salt
    ) public pure returns (bytes32) {
        return keccak256(abi.encode(targets, values, payloads, predecessor, salt));
    }

    function isOperation(bytes32 id) public view returns (bool) {
        return _timestamps[id] > 0;
    }

    function isOperationReady(bytes32 id) public view returns (bool) {
        uint256 ts = _timestamps[id];
        return ts > 1 && ts <= block.timestamp;
    }

    function isOperationDone(bytes32 id) public view returns (bool) {
        return _timestamps[id] == 1;
    }

    function scheduleBatch(
        address[] calldata targets,
        uint256[] calldata values,
        bytes[] calldata payloads,
        bytes32 predecessor,
        bytes32 salt,
        uint256 delay
    ) external {
        require(_roles[PROPOSER_ROLE][msg.sender], "TimelockController: caller must have proposer role");
        require(delay >= _minDelay, "TimelockController: insufficient delay");

        bytes32 id = hashOperationBatch(targets, values, payloads, predecessor, salt);
        require(!isOperation(id), "TimelockController: operation already scheduled");

        _timestamps[id] = block.timestamp + delay;

        for (uint256 i = 0; i < targets.length; i++) {
            emit CallScheduled(id, i, targets[i], values[i], payloads[i], predecessor, delay);
        }
    }

    function executeBatch(
        address[] calldata targets,
        uint256[] calldata values,
        bytes[] calldata payloads,
        bytes32 predecessor,
        bytes32 salt
    ) external payable {
        require(_roles[EXECUTOR_ROLE][msg.sender], "TimelockController: caller must have executor role");

        bytes32 id = hashOperationBatch(targets, values, payloads, predecessor, salt);
        require(isOperationReady(id), "TimelockController: operation is not ready");

        if (predecessor != bytes32(0)) {
            require(isOperationDone(predecessor), "TimelockController: missing dependency");
        }

        for (uint256 i = 0; i < targets.length; i++) {
            (bool success,) = targets[i].call{value: values[i]}(payloads[i]);
            require(success, "TimelockController: underlying transaction reverted");
            emit CallExecuted(id, i, targets[i], values[i], payloads[i]);
        }

        _timestamps[id] = 1; // Mark as done
    }
}
