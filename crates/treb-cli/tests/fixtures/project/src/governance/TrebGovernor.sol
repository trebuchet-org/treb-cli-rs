// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

/// @title Minimal Governor stub for treb integration tests.
/// @dev Implements only the subset of OZ Governor needed by the routing pipeline:
///      propose() with selector 0x7d5e81e2, state() returning uint8 0-7,
///      and hashProposal() for proposal ID derivation.
///      Voting is simplified — castVote always counts 1 vote per address.
contract TrebGovernor {
    /// @dev Matches OZ Governor ProposalState enum (0-7).
    ///      state() returns uint8 cast from these values.
    enum ProposalState {
        Pending,    // 0
        Active,     // 1
        Canceled,   // 2
        Defeated,   // 3
        Succeeded,  // 4
        Queued,     // 5
        Expired,    // 6
        Executed    // 7
    }

    struct ProposalCore {
        uint64 voteStart;
        uint64 voteEnd;
        bool executed;
        bool canceled;
        bool queued;
    }

    address public token;
    address public timelock;
    uint256 public votingDelay;
    uint256 public votingPeriod;
    uint256 public quorumThreshold;

    mapping(uint256 => ProposalCore) private _proposals;
    mapping(uint256 => mapping(address => bool)) private _hasVoted;
    mapping(uint256 => uint256) private _forVotes;
    mapping(uint256 => uint256) private _againstVotes;

    event ProposalCreated(
        uint256 proposalId,
        address proposer,
        address[] targets,
        uint256[] values,
        string[] signatures,
        bytes[] calldatas,
        uint256 voteStart,
        uint256 voteEnd,
        string description
    );
    event ProposalExecuted(uint256 proposalId);
    event ProposalCanceled(uint256 proposalId);
    event VoteCast(address indexed voter, uint256 proposalId, uint8 support, uint256 weight, string reason);

    constructor(
        address _token,
        address _timelock,
        uint256 _votingDelay,
        uint256 _votingPeriod,
        uint256 _quorumThreshold
    ) {
        token = _token;
        timelock = _timelock;
        votingDelay = _votingDelay;
        votingPeriod = _votingPeriod;
        quorumThreshold = _quorumThreshold;
    }

    /// @dev Hash proposal parameters to derive the proposal ID.
    ///      Matches OZ Governor.hashProposal() exactly.
    function hashProposal(
        address[] memory targets,
        uint256[] memory values,
        bytes[] memory calldatas,
        bytes32 descriptionHash
    ) public pure returns (uint256) {
        return uint256(keccak256(abi.encode(targets, values, calldatas, descriptionHash)));
    }

    /// @notice Create a new proposal.
    /// @dev Selector: 0x7d5e81e2 — matches OZ Governor.propose().
    function propose(
        address[] memory targets,
        uint256[] memory values,
        bytes[] memory calldatas,
        string memory description
    ) public returns (uint256) {
        bytes32 descHash = keccak256(bytes(description));
        uint256 proposalId = hashProposal(targets, values, calldatas, descHash);

        require(_proposals[proposalId].voteStart == 0, "Governor: proposal already exists");

        uint64 snapshot = uint64(block.number + votingDelay);
        uint64 deadline = snapshot + uint64(votingPeriod);

        _proposals[proposalId] = ProposalCore({
            voteStart: snapshot,
            voteEnd: deadline,
            executed: false,
            canceled: false,
            queued: false
        });

        string[] memory sigs = new string[](targets.length);
        emit ProposalCreated(
            proposalId, msg.sender, targets, values, sigs, calldatas,
            snapshot, deadline, description
        );

        return proposalId;
    }

    /// @notice Query proposal state.
    /// @dev Selector: 0x3e4f49e6 — matches OZ Governor.state().
    ///      Returns uint8 values 0-7 matching map_onchain_state() in governor.rs.
    function state(uint256 proposalId) public view returns (uint8) {
        ProposalCore storage proposal = _proposals[proposalId];
        require(proposal.voteStart > 0, "Governor: unknown proposal id");

        if (proposal.executed) return uint8(ProposalState.Executed);   // 7
        if (proposal.canceled) return uint8(ProposalState.Canceled);   // 2

        if (block.number < proposal.voteStart) return uint8(ProposalState.Pending);  // 0
        if (block.number <= proposal.voteEnd) return uint8(ProposalState.Active);    // 1

        // Voting ended — check quorum + majority
        if (proposal.queued) return uint8(ProposalState.Queued);  // 5

        if (_forVotes[proposalId] >= quorumThreshold && _forVotes[proposalId] > _againstVotes[proposalId]) {
            return uint8(ProposalState.Succeeded);  // 4
        }

        return uint8(ProposalState.Defeated);  // 3
    }

    /// @dev Simplified voting — 1 vote per address (does not check token delegation).
    function castVote(uint256 proposalId, uint8 support) external returns (uint256) {
        require(state(proposalId) == uint8(ProposalState.Active), "Governor: vote not currently active");
        require(!_hasVoted[proposalId][msg.sender], "Governor: vote already cast");

        _hasVoted[proposalId][msg.sender] = true;
        uint256 weight = 1;

        if (support == 1) {
            _forVotes[proposalId] += weight;
        } else if (support == 0) {
            _againstVotes[proposalId] += weight;
        }

        emit VoteCast(msg.sender, proposalId, support, weight, "");
        return weight;
    }
}
