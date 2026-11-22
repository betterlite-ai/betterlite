use std::future::Future;

use crate::agent_policy::AgentResponse;
use crate::identity::AgentIdentityFailure;

// The server backs this with B4773's AgentIdentityStore, whose agent and retired_agent answer the same way
pub trait AdmissionStore: Sync {
    fn agent(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<AgentResponse>, AgentIdentityFailure>> + Send;

    fn retired_agent(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<bool, AgentIdentityFailure>> + Send;
}

// B8645 backs this with B8231's agents read-through: reload one agent into the registry, true when it may have changed
pub trait ReadThrough: Sync {
    fn attempt(&self, agent_id_or_name: &str) -> impl Future<Output = bool> + Send;
}

// the sources get_agent_with_read_through takes from the proxy, which prepare_agent_invocation receives together
pub struct InvocationSources<'a, Store, Read> {
    pub store: Option<&'a Store>,
    pub read_through: &'a Read,
}
