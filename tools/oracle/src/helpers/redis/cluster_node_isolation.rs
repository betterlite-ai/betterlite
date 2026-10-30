use betterlite_redis::redis_cluster_node_isolation::{
    CommandOutcome, IsolationBranch, IsolationConfig, IsolationError, IsolationState,
    execute_command, select_branch, version_warning,
};

#[derive(serde::Deserialize)]
#[serde(transparent)]
pub(crate) struct BoolFlag(bool);

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    pub version: String,
    #[serde(rename = "has_update_active_connections_for_reconnect")]
    pub reconnect_method: BoolFlag,
    pub node: String,
    pub ttl: u32,
    pub reinitialize_steps: u32,
    pub initialize: BoolFlag,
    pub read_from_replicas: BoolFlag,
    pub outcomes: Vec<CommandOutcome>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let branch = select_branch(input.reconnect_method.0);
        let mut state = IsolationState::new(
            IsolationConfig {
                branch,
                request_ttl: input.ttl,
                reinitialize_steps: input.reinitialize_steps,
                initialize: input.initialize.0,
                read_from_replicas: input.read_from_replicas.0,
            },
            &input.node,
        );
        let result = execute_command(&mut state, &input.node, &input.outcomes);
        let (status, node, events) = match result {
            Ok(command_result) => ("success", Some(command_result.node), command_result.events),
            Err(error) => (error_name(&error), None, Vec::new()),
        };
        Ok(serde_json::json!({
            "branch": branch_name(branch),
            "status": status,
            "node": node,
            "events": events.iter().map(event_name).collect::<Vec<_>>(),
            "disconnected_nodes": state.disconnected,
            "reinitialize_counter": state.reinitialize_counter,
            "sleeps": state.sleeps,
            "read_from_replicas": state.read_from_replicas,
            "version_warning": version_warning(branch, &input.version),
        }))
    }
}

const fn branch_name(branch: IsolationBranch) -> &'static str {
    match branch {
        IsolationBranch::Legacy => "legacy",
        IsolationBranch::PerNodeRecovery => "per_node_recovery",
    }
}

const fn error_name(error: &IsolationError) -> &'static str {
    match error {
        IsolationError::Connection => "connection",
        IsolationError::Timeout => "timeout",
        IsolationError::ClusterDown => "cluster_down",
        IsolationError::SlotNotCovered => "slot_not_covered",
        IsolationError::BusyLoading => "busy_loading",
        IsolationError::MaxConnections => "max_connections",
        IsolationError::TtlExhausted => "ttl_exhausted",
    }
}

fn event_name(event: &betterlite_redis::redis_cluster_node_isolation::IsolationEvent) -> String {
    match event {
        betterlite_redis::redis_cluster_node_isolation::IsolationEvent::Disconnected(node) => {
            format!("disconnected:{node}")
        }
        betterlite_redis::redis_cluster_node_isolation::IsolationEvent::Moved => "moved".to_owned(),
        betterlite_redis::redis_cluster_node_isolation::IsolationEvent::Asked(node) => {
            format!("asked:{node}")
        }
        betterlite_redis::redis_cluster_node_isolation::IsolationEvent::FullReinitialize => {
            "full_reinitialize".to_owned()
        }
        betterlite_redis::redis_cluster_node_isolation::IsolationEvent::Slept(seconds) => {
            format!("slept:{seconds}")
        }
    }
}
