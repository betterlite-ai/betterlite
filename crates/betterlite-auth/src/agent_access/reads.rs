use std::collections::BTreeSet;

use secrecy::SecretString;

use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::AgentResponse;
use crate::identity::AgentIdentityFailure;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::error::AgentAccessError;

// The server backs this with B4779's key and team grant methods and B4778's managed ceilings
pub trait GrantSources: Sync {
    fn key_team_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send;

    fn caller_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send;

    fn key_access_strict(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send;

    // already mapped to stable ids; None when the key is no agent key or its agent lists no group
    fn access_group_ceiling(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> impl Future<Output = Result<Option<BTreeSet<String>>, AgentAccessError>> + Send;

    // one set of agent ids per ceiling, raw ids as the groups store them
    fn managed_ceilings(
        &self,
        agent_id: &str,
        access_group_ids: &[String],
    ) -> impl Future<Output = Result<Vec<BTreeSet<String>>, AgentAccessError>> + Send;
}

// The server backs this with B4773's AgentIdentityStore
pub trait TargetAgents: Sync {
    fn has_database(&self) -> bool;

    // a failed read is the policy_unavailable failure, a missing row is Ok(None)
    fn agent(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<AgentResponse>, AgentIdentityFailure>> + Send;
}

// The server backs this with B4679's key reload, B4673's user reload, B4671's sources and B4672's contexts
pub trait HumanSources: Sync {
    fn reload_admitted_key(
        &self,
        key_hash: &SecretString,
        check_db_only: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AgentAccessError>> + Send;

    fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AgentAccessError>> + Send;

    // one auth per grant source, the user's own source has no team_id; None admits every team
    fn admitted_subject_sources(
        &self,
        human: &UserAPIKeyAuth,
        allowed_team_ids: Option<&BTreeSet<String>>,
    ) -> impl Future<Output = Result<Vec<UserAPIKeyAuth>, AgentAccessError>> + Send;

    fn effective_contexts(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Vec<UserAPIKeyAuth>, AgentAccessError>> + Send;
}
