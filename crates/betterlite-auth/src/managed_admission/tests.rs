#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::agent_access::error::AgentAccessError;
use crate::agent_access::reads::{GrantSources, HumanSources, TargetAgents};
use crate::agent_access::resolver::AgentAccessResolver;
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::{AgentResponse, ManagedAgentContext, ManagedAgentMode};
use crate::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use crate::identity::AgentIdentityFailure;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

use super::error::AdmissionError;
use super::ports::{AdmissionStore, ReadThrough};
use super::{managed_marks, managed_token};

pub(super) const MANAGED: &str = r#"{"agent_id":"agent-1","agent_name":"managed-agent","agent_card_params":{},"identity_managed":true,"execution_mode":"delegated","identity":{"agent_id":"agent-1","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}"#;
pub(super) const PLAIN: &str =
    r#"{"agent_id":"plain","agent_name":"plain-agent","agent_card_params":{}}"#;

pub(super) fn agent(fields: &str) -> AgentResponse {
    serde_json::from_str(fields).unwrap()
}

pub(super) fn key(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).unwrap()
}

pub(super) fn context(mode: ManagedAgentMode, user_id: Option<&str>) -> ManagedAgentContext {
    ManagedAgentContext {
        agent_id: String::from("agent-1"),
        binding_revision: Some(String::from("rev-1")),
        mode,
        user_id: user_id.map(str::to_owned),
        subject_oid: None,
    }
}

pub(super) fn registry(agents: &[&str]) -> Arc<AgentRegistry> {
    let registry = AgentRegistry::default();
    for fields in agents {
        registry.register_agent(agent(fields));
    }
    Arc::new(registry)
}

pub(super) fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

// a refusal as its status and detail, the two things LiteLLM's raise_identity_failure decides
pub(super) fn refusal(outcome: Result<(), AdmissionError>) -> (u16, String) {
    let error = outcome.unwrap_err();
    (error.status, error.detail)
}

pub(super) fn access_failure(status: u16) -> AgentAccessError {
    AgentAccessError::Rejected {
        status,
        detail: String::from("the case port failed"),
    }
}

#[derive(Default)]
pub(super) struct FakeStore {
    pub(super) agents: Vec<AgentResponse>,
    pub(super) retired: bool,
    pub(super) failure: Option<AgentIdentityFailure>,
    pub(super) retired_failure: Option<AgentIdentityFailure>,
}

impl AdmissionStore for FakeStore {
    fn agent(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<AgentResponse>, AgentIdentityFailure>> + Send {
        if let Some(failure) = self.failure.clone() {
            return ready(Err(failure));
        }
        let found = self
            .agents
            .iter()
            .find(|agent| agent.agent_id == agent_id)
            .cloned();
        ready(Ok(found))
    }

    fn retired_agent(
        &self,
        _agent_id: &str,
    ) -> impl Future<Output = Result<bool, AgentIdentityFailure>> + Send {
        ready(self.retired_failure.clone().map_or(Ok(self.retired), Err))
    }
}

#[derive(Default)]
pub(super) struct MissedReadThrough {
    pub(super) attempts: Mutex<Vec<String>>,
}

impl ReadThrough for MissedReadThrough {
    fn attempt(&self, agent_id_or_name: &str) -> impl Future<Output = bool> + Send {
        self.attempts
            .lock()
            .unwrap()
            .push(agent_id_or_name.to_owned());
        ready(false)
    }
}

pub(super) struct RegisteringReadThrough {
    pub(super) registry: Arc<AgentRegistry>,
    pub(super) agent: AgentResponse,
}

impl ReadThrough for RegisteringReadThrough {
    fn attempt(&self, _agent_id_or_name: &str) -> impl Future<Output = bool> + Send {
        self.registry.register_agent(self.agent.clone());
        ready(true)
    }
}

#[derive(Default)]
pub(super) struct FakeGrants {
    pub(super) granted: BTreeSet<String>,
    pub(super) failure: Option<u16>,
}

impl GrantSources for FakeGrants {
    fn key_team_access(
        &self,
        _auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send {
        let answer = match self.failure {
            Some(status) => Err(access_failure(status)),
            None => Ok(AgentAccess::Restricted(self.granted.clone())),
        };
        ready(answer)
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

#[derive(Default)]
pub(super) struct FakeHumans {
    pub(super) failure: Option<u16>,
}

impl HumanSources for FakeHumans {
    fn reload_admitted_key(
        &self,
        _key_hash: &secrecy::SecretString,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AgentAccessError>> + Send {
        ready(Err(access_failure(500)))
    }

    fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AgentAccessError>> + Send {
        if let Some(status) = self.failure {
            return ready(Err(access_failure(status)));
        }
        if !requires_fresh_policy {
            return ready(Err(access_failure(500)));
        }
        ready(Ok(key(&format!(r#"{{"user_id":"{user_id}"}}"#))))
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

#[derive(Default)]
pub(super) struct FakeTargets {
    pub(super) agent: Option<AgentResponse>,
    pub(super) registry: Option<Arc<AgentRegistry>>,
}

impl FakeTargets {
    fn known(&self, agent_id: &str) -> Option<AgentResponse> {
        self.agent
            .as_ref()
            .filter(|agent| agent.agent_id == agent_id)
            .cloned()
            .or_else(|| {
                self.registry
                    .as_ref()
                    .and_then(|registry| registry.get_agent_by_id(agent_id))
            })
    }
}

impl TargetAgents for FakeTargets {
    fn has_database(&self) -> bool {
        true
    }

    fn agent(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<AgentResponse>, AgentIdentityFailure>> + Send {
        ready(Ok(self.known(agent_id)))
    }
}

pub(super) type Resolver = AgentAccessResolver<FakeGrants, FakeTargets, FakeHumans>;

pub(super) fn resolver(
    registry: &Arc<AgentRegistry>,
    granted: &[&str],
    target: Option<AgentResponse>,
) -> Resolver {
    AgentAccessResolver {
        registry: Arc::clone(registry),
        grants: FakeGrants {
            granted: ids(granted),
            failure: None,
        },
        targets: FakeTargets {
            agent: target,
            registry: Some(Arc::clone(registry)),
        },
        humans: FakeHumans::default(),
    }
}

#[test]
fn managed_token_keeps_the_context_the_key_view_drops() {
    let mut fields = BTreeMap::new();
    fields.insert(
        String::from("user_id"),
        JsonData::String(String::from("human")),
    );
    fields.insert(
        String::from("agent_id"),
        JsonData::String(String::from("agent-1")),
    );
    let auth = managed_token(fields, Some(context(ManagedAgentMode::Autonomous, None))).unwrap();
    assert_eq!(auth.view.token.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(
        auth.extra.managed_agent_context.map(|given| given.agent_id),
        Some(String::from("agent-1"))
    );
}

#[test]
fn managed_marks_read_the_policy_the_fresh_flag_and_the_price() {
    let mut auth = key(r#"{"user_id":"human"}"#);
    auth.extra.billing_agent_policy = Some(agent(PLAIN));
    auth.extra.requires_fresh_policy = true;
    auth.extra.agent_invocation_cost = Some(Decimal::ONE);
    let marks = managed_marks(&auth);
    assert_eq!(
        marks.billing_agent_policy.map(|agent| agent.agent_id),
        Some(String::from("plain"))
    );
    assert!(marks.requires_fresh_policy);
    assert_eq!(marks.agent_invocation_cost, Some(Decimal::ONE));
}
