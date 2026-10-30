mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

const TIMEOUT_REINIT_LIMIT: u32 = 3;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IsolationBranch {
    Legacy,
    PerNodeRecovery,
}

#[must_use]
pub fn version_warning(version: &str) -> Option<String> {
    if version == "5.3.1" {
        None
    } else {
        Some(format!(
            "redis-py {version} is not a verified cluster-isolation version"
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandOutcome {
    Success,
    ConnectionError,
    Timeout,
    Moved,
    Ask { node: String },
    ClusterDown,
    SlotNotCovered,
    TryAgain,
    BusyLoading,
    MaxConnections,
}

#[derive(Clone, Debug, PartialEq)]
pub enum IsolationEvent {
    Disconnected(String),
    Moved,
    Asked(String),
    FullReinitialize,
    Slept(f64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IsolationError {
    Connection,
    Timeout,
    ClusterDown,
    SlotNotCovered,
    BusyLoading,
    MaxConnections,
    TtlExhausted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandResult {
    pub node: String,
    pub events: Vec<IsolationEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IsolationConfig {
    pub branch: IsolationBranch,
    pub request_ttl: u32,
    pub reinitialize_steps: u32,
    pub initialize: bool,
    pub read_from_replicas: bool,
}

#[derive(Clone, Debug)]
pub struct IsolationState {
    pub branch: IsolationBranch,
    pub request_ttl: u32,
    pub reinitialize_steps: u32,
    pub reinitialize_counter: u32,
    pub initialize: bool,
    pub tolerated_timeouts: u32,
    pub timeout_counts: BTreeMap<String, u32>,
    pub disconnected: Vec<String>,
    pub sleeps: Vec<f64>,
    pub default_node: String,
    pub read_from_replicas: bool,
}

impl IsolationState {
    #[must_use]
    pub fn new(config: IsolationConfig, default_node: &str) -> Self {
        let branch = config.branch;
        Self {
            branch,
            request_ttl: config.request_ttl,
            reinitialize_steps: config.reinitialize_steps,
            reinitialize_counter: 0,
            initialize: config.initialize,
            tolerated_timeouts: 0,
            timeout_counts: BTreeMap::new(),
            disconnected: Vec::new(),
            sleeps: Vec::new(),
            default_node: default_node.to_owned(),
            read_from_replicas: config.read_from_replicas,
        }
    }

    fn disconnect(&mut self, node: &str, events: &mut Vec<IsolationEvent>) {
        self.disconnected.push(node.to_owned());
        events.push(IsolationEvent::Disconnected(node.to_owned()));
    }

    fn reinitialize(&mut self, events: &mut Vec<IsolationEvent>) {
        self.initialize = true;
        events.push(IsolationEvent::FullReinitialize);
    }

    fn handle_timeout(
        &mut self,
        node: &str,
        events: &mut Vec<IsolationEvent>,
    ) -> Result<(), IsolationError> {
        if self.branch == IsolationBranch::Legacy {
            self.disconnect(node, events);
            return Err(IsolationError::Timeout);
        }
        let count = self.timeout_counts.entry(node.to_owned()).or_insert(0);
        *count = count.saturating_add(1);
        if *count >= TIMEOUT_REINIT_LIMIT {
            self.timeout_counts.remove(node);
            self.initialize = true;
        } else {
            self.tolerated_timeouts = self.tolerated_timeouts.saturating_add(1);
        }
        Err(IsolationError::Timeout)
    }

    fn handle_moved(&mut self, events: &mut Vec<IsolationEvent>) {
        self.reinitialize_counter = self.reinitialize_counter.saturating_add(1);
        if self.reinitialize_steps != 0
            && self
                .reinitialize_counter
                .is_multiple_of(self.reinitialize_steps)
        {
            self.reinitialize_counter = 0;
            self.reinitialize(events);
        }
        events.push(IsolationEvent::Moved);
    }
}

#[must_use]
pub const fn select_branch(has_reconnect_method: bool) -> IsolationBranch {
    if has_reconnect_method {
        IsolationBranch::PerNodeRecovery
    } else {
        IsolationBranch::Legacy
    }
}

pub fn execute_command(
    state: &mut IsolationState,
    initial_node: &str,
    outcomes: &[CommandOutcome],
) -> Result<CommandResult, IsolationError> {
    let mut context = CommandContext {
        node: initial_node.to_owned(),
        redirect_node: state.default_node.clone(),
        moved: false,
        asking: false,
        events: Vec::new(),
    };
    let mut outcome_index = 0_usize;
    let original_ttl = state.request_ttl;
    while state.request_ttl > 0 {
        state.request_ttl = state
            .request_ttl
            .checked_sub(1)
            .ok_or(IsolationError::TtlExhausted)?;
        context.prepare(initial_node);
        let outcome = outcomes
            .get(outcome_index)
            .ok_or(IsolationError::TtlExhausted)?;
        outcome_index = outcome_index
            .checked_add(1)
            .ok_or(IsolationError::TtlExhausted)?;
        if apply_outcome(state, &mut context, outcome, original_ttl)? {
            return Ok(CommandResult {
                node: context.node,
                events: context.events,
            });
        }
    }
    Err(IsolationError::TtlExhausted)
}

struct CommandContext {
    node: String,
    redirect_node: String,
    moved: bool,
    asking: bool,
    events: Vec<IsolationEvent>,
}

impl CommandContext {
    fn prepare(&mut self, initial_node: &str) {
        if self.asking {
            self.node.clone_from(&self.redirect_node);
            self.asking = false;
            self.events.push(IsolationEvent::Asked(self.node.clone()));
            return;
        }
        if self.moved {
            initial_node.clone_into(&mut self.node);
            self.moved = false;
        }
    }
}

fn apply_outcome(
    state: &mut IsolationState,
    context: &mut CommandContext,
    outcome: &CommandOutcome,
    original_ttl: u32,
) -> Result<bool, IsolationError> {
    match outcome {
        CommandOutcome::Success => {
            state.timeout_counts.remove(&context.node);
            Ok(true)
        }
        CommandOutcome::ConnectionError => {
            state.disconnect(&context.node, &mut context.events);
            Err(IsolationError::Connection)
        }
        CommandOutcome::Timeout => {
            state.handle_timeout(&context.node, &mut context.events)?;
            Err(IsolationError::Timeout)
        }
        CommandOutcome::Moved => {
            state.handle_moved(&mut context.events);
            context.moved = true;
            Ok(false)
        }
        CommandOutcome::Ask { node: target } => {
            context.redirect_node.clone_from(target);
            context.asking = true;
            Ok(false)
        }
        CommandOutcome::ClusterDown => {
            sleep_for_topology_change(state, context, IsolationError::ClusterDown)
        }
        CommandOutcome::SlotNotCovered => {
            sleep_for_topology_change(state, context, IsolationError::SlotNotCovered)
        }
        CommandOutcome::TryAgain => {
            if state.request_ttl.saturating_mul(2) < original_ttl {
                state.sleeps.push(0.05_f64);
                context.events.push(IsolationEvent::Slept(0.05_f64));
            }
            Ok(false)
        }
        CommandOutcome::BusyLoading => Err(IsolationError::BusyLoading),
        CommandOutcome::MaxConnections => Err(IsolationError::MaxConnections),
    }
}

fn sleep_for_topology_change(
    state: &mut IsolationState,
    context: &mut CommandContext,
    error: IsolationError,
) -> Result<bool, IsolationError> {
    state.reinitialize(&mut context.events);
    state.sleeps.push(0.25_f64);
    context.events.push(IsolationEvent::Slept(0.25_f64));
    Err(error)
}
