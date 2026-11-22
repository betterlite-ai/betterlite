use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_auth::agent_access::error::AgentAccessError;
use betterlite_auth::agent_access::reads::{GrantSources, HumanSources, TargetAgents};
use betterlite_auth::agent_access::resolver::AgentAccessResolver;
use betterlite_auth::agent_access_groups::access::AgentAccess;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_auth::identity::AgentIdentityFailure;
use betterlite_auth::managed_admission::ports::{AdmissionStore, ReadThrough};
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;

// a case port that cannot answer raises the same 500 a broken real port would
fn case_failure(detail: String) -> AgentAccessError {
    AgentAccessError::Rejected {
        status: 500,
        detail,
    }
}

fn key(user_id: &str) -> Result<UserAPIKeyAuth, AuthError> {
    let mut fields = BTreeMap::new();
    fields.insert(
        String::from("user_id"),
        JsonData::String(user_id.to_owned()),
    );
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
}

// LiteLLM's AgentIdentityStore over the case rows: the agent table, then the retired table
pub(super) struct CaseStore {
    pub(super) agents: Vec<AgentResponse>,
    pub(super) retired: bool,
}

impl AdmissionStore for CaseStore {
    fn agent(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<AgentResponse>, AgentIdentityFailure>> + Send {
        let found = self.agents.iter().find(|agent| agent.agent_id == agent_id);
        ready(Ok(found.cloned()))
    }

    fn retired_agent(
        &self,
        _agent_id: &str,
    ) -> impl Future<Output = Result<bool, AgentIdentityFailure>> + Send {
        ready(Ok(self.retired))
    }
}

// the case target is already registered, so get_agent_with_read_through never reloads one
pub(super) struct NoReadThrough;

impl ReadThrough for NoReadThrough {
    fn attempt(&self, _agent_id_or_name: &str) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

// LiteLLM's resolve_access: the case's grants are Restricted, so strict mode keeps them
pub(super) struct CaseGrants {
    pub(super) granted: BTreeSet<String>,
}

impl GrantSources for CaseGrants {
    fn key_team_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send {
        let access = if auth.is_some() {
            AgentAccess::Restricted(self.granted.clone())
        } else {
            AgentAccess::Unrestricted
        };
        ready(Ok(access))
    }

    fn caller_access(
        &self,
        _auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send {
        ready(Ok(AgentAccess::Unrestricted))
    }

    fn key_access_strict(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send {
        ready(Ok(AgentAccess::Restricted(self.granted.clone())))
    }

    fn access_group_ceiling(
        &self,
        _auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> impl Future<Output = Result<Option<BTreeSet<String>>, AgentAccessError>> + Send {
        ready(Ok(None))
    }

    fn managed_ceilings(
        &self,
        _agent_id: &str,
        _access_group_ids: &[String],
    ) -> impl Future<Output = Result<Vec<BTreeSet<String>>, AgentAccessError>> + Send {
        ready(Ok(Vec::new()))
    }
}

// the store's rows and the registry's copies, the two places is_agent_allowed reads a target from
pub(super) struct CaseTargets {
    pub(super) store: Vec<AgentResponse>,
    pub(super) registry: Arc<AgentRegistry>,
    pub(super) database: bool,
}

impl TargetAgents for CaseTargets {
    fn has_database(&self) -> bool {
        self.database
    }

    fn agent(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<AgentResponse>, AgentIdentityFailure>> + Send {
        let found = self
            .store
            .iter()
            .find(|agent| agent.agent_id == agent_id)
            .cloned()
            .or_else(|| self.registry.get_agent_by_id(agent_id));
        ready(Ok(found))
    }
}

// LiteLLM's effective_contexts, reload_admitted_user and admitted_subject_sources: one human, one own source
pub(super) struct CaseHumans;

impl HumanSources for CaseHumans {
    fn reload_admitted_key(
        &self,
        _key_hash: &SecretString,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AgentAccessError>> + Send {
        ready(Err(case_failure(String::from("the cases reload no key"))))
    }

    fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AgentAccessError>> + Send {
        if !requires_fresh_policy {
            return ready(Err(case_failure(String::from(
                "the human is reloaded without a fresh policy",
            ))));
        }
        ready(key(user_id).map_err(|error| case_failure(error.to_string())))
    }

    fn admitted_subject_sources(
        &self,
        human: &UserAPIKeyAuth,
        _allowed_team_ids: Option<&BTreeSet<String>>,
    ) -> impl Future<Output = Result<Vec<UserAPIKeyAuth>, AgentAccessError>> + Send {
        ready(Ok(vec![human.clone()]))
    }

    fn effective_contexts(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Vec<UserAPIKeyAuth>, AgentAccessError>> + Send {
        ready(Ok(vec![auth.clone()]))
    }
}

pub(super) type CaseResolver = AgentAccessResolver<CaseGrants, CaseTargets, CaseHumans>;
