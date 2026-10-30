#![cfg(test)]

use rstest::rstest;

use super::{
    CommandOutcome, IsolationBranch, IsolationConfig, IsolationError, IsolationEvent,
    IsolationState, execute_command, select_branch,
};

#[test]
fn redis_cluster_node_isolation_behaviors() {
    isolated_timeout_disconnects_only_one_node();
    redirects_keep_the_target_node();
}

fn isolated_timeout_disconnects_only_one_node() {
    let mut state = state(IsolationBranch::Legacy, 4, 0, false);
    let result = execute_command(
        &mut state,
        "a",
        &[CommandOutcome::Timeout, CommandOutcome::Success],
    );
    assert_eq!(result, Err(IsolationError::Timeout));
    assert_eq!(state.disconnected, ["a"]);
}

fn redirects_keep_the_target_node() {
    let mut state = state(IsolationBranch::Legacy, 4, 0, true);
    assert!(state.read_from_replicas);
    let result = execute_command(
        &mut state,
        "a",
        &[
            CommandOutcome::Moved,
            CommandOutcome::Ask {
                node: "b".to_owned(),
            },
            CommandOutcome::Success,
        ],
    )
    .unwrap();
    assert_eq!(
        result.events,
        [IsolationEvent::Moved, IsolationEvent::Asked("b".to_owned())]
    );
    assert_eq!(result.node, "b");
    assert_eq!(state.reinitialize_counter, 1);
}

#[rstest]
#[case::legacy(false, IsolationBranch::Legacy)]
#[case::newer(true, IsolationBranch::PerNodeRecovery)]
fn redis_cluster_node_isolation_boundaries(
    #[case] has_reconnect_method: bool,
    #[case] expected: IsolationBranch,
) {
    assert_eq!(select_branch(has_reconnect_method), expected);
    let mut ttl_state = state(expected, 2, 0, false);
    let ttl_result = execute_command(
        &mut ttl_state,
        "a",
        &[CommandOutcome::TryAgain, CommandOutcome::TryAgain],
    );
    assert_eq!(ttl_result, Err(IsolationError::TtlExhausted));
    assert_eq!(ttl_state.sleeps, [0.05_f64]);

    let mut reinit_state = state(expected, 3, 1, false);
    let reinit_result = execute_command(
        &mut reinit_state,
        "a",
        &[CommandOutcome::Moved, CommandOutcome::Success],
    )
    .unwrap();
    assert!(
        reinit_result
            .events
            .contains(&IsolationEvent::FullReinitialize)
    );
}

fn state(
    branch: IsolationBranch,
    request_ttl: u32,
    reinitialize_steps: u32,
    read_from_replicas: bool,
) -> IsolationState {
    IsolationState::new(
        IsolationConfig {
            branch,
            request_ttl,
            reinitialize_steps,
            initialize: false,
            read_from_replicas,
        },
        "a",
    )
}
