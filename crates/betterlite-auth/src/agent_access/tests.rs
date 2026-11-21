#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::AgentAccessError;
use super::reads::{GrantSources, HumanSources, TargetAgents};
use super::resolver::AgentAccessResolver;
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::AgentResponse;
use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::identity::AgentIdentityFailure;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(super) const ALPHA_STABLE: &str =
    "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8";
pub(super) const ALPHA_LEGACY: &str =
    "00afdb1e315efd7d8d96382f3925c7d46dee40711cffe80449b0dcb21f731322";

// Err is the status of the Rejected error the fake port answers with
pub(super) type Answer<T> = Result<T, u16>;
pub(super) type FakeResolver = AgentAccessResolver<FakeGrants, FakeTargets, FakeHumans>;

pub(super) fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn restricted(values: &[&str]) -> AgentAccess {
    AgentAccess::Restricted(ids(values))
}

pub(super) fn auth(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).unwrap()
}

pub(super) fn agent(fields: &str) -> AgentResponse {
    serde_json::from_str(fields).unwrap()
}

// an identity-managed row whose identity is enabled and active
pub(super) fn managed_row(agent_id: &str) -> AgentResponse {
    agent(&format!(
        r#"{{"agent_id":"{agent_id}","agent_name":"{agent_id}","agent_card_params":{{}},"identity_managed":true,"identity":{{"agent_id":"{agent_id}","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}}}"#
    ))
}

pub(super) fn plain_agent(agent_id: &str) -> AgentResponse {
    agent(&format!(
        r#"{{"agent_id":"{agent_id}","agent_name":"{agent_id}","agent_card_params":{{}}}}"#
    ))
}

pub(super) fn alpha_registry() -> Arc<AgentRegistry> {
    let alpha: Json =
        serde_json::from_str(r#"{"agent_name":"alpha","agent_card_params":{"model":"gpt-4o"}}"#)
            .unwrap();
    let registry = AgentRegistry::default();
    registry
        .load_agents_from_config(Some([alpha].as_slice()))
        .unwrap();
    Arc::new(registry)
}

pub(super) fn rejected_status<Answered>(outcome: Result<Answered, AgentAccessError>) -> u16 {
    match outcome {
        Ok(_) => panic!("the call was expected to fail"),
        Err(error) => error.status(),
    }
}

fn answer<T: Clone>(given: &Answer<T>) -> Result<T, AgentAccessError> {
    given.clone().map_err(|status| AgentAccessError::Rejected {
        status,
        detail: format!("fake port failed with {status}"),
    })
}

// which source or context a fake call is for: its team, else its user
pub(super) fn label(auth: Option<&UserAPIKeyAuth>) -> String {
    let Some(key) = auth else {
        return String::from("none");
    };
    let token = &key.view.token;
    token
        .team_id
        .clone()
        .or_else(|| token.user_id.clone())
        .unwrap_or_else(|| String::from("anonymous"))
}

pub(super) fn calls(log: &Mutex<Vec<String>>) -> Vec<String> {
    log.lock().unwrap().clone()
}

pub(super) struct FakeGrants {
    // keyed by label; a label without an entry answers Unrestricted
    pub(super) key_team: BTreeMap<String, Answer<AgentAccess>>,
    pub(super) caller: Answer<AgentAccess>,
    pub(super) key_strict: Answer<AgentAccess>,
    pub(super) ceiling: Answer<Option<BTreeSet<String>>>,
    pub(super) managed_ceilings: Answer<Vec<BTreeSet<String>>>,
    pub(super) calls: Mutex<Vec<String>>,
}

impl Default for FakeGrants {
    fn default() -> Self {
        Self {
            key_team: BTreeMap::new(),
            caller: Ok(AgentAccess::Unrestricted),
            key_strict: Ok(AgentAccess::Unrestricted),
            ceiling: Ok(None),
            managed_ceilings: Ok(Vec::new()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl GrantSources for FakeGrants {
    async fn key_team_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> Result<AgentAccess, AgentAccessError> {
        let source = label(auth);
        let call = format!("key_team {source} strict={strict}");
        self.calls.lock().unwrap().push(call);
        self.key_team
            .get(&source)
            .map_or(Ok(AgentAccess::Unrestricted), answer)
    }

    async fn caller_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> Result<AgentAccess, AgentAccessError> {
        let call = format!("caller {} strict={strict}", label(auth));
        self.calls.lock().unwrap().push(call);
        answer(&self.caller)
    }

    async fn key_access_strict(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> Result<AgentAccess, AgentAccessError> {
        let permission = auth.view.token.object_permission.as_ref();
        let declared = permission.map(|table| table.agents.0.clone());
        let call = format!("key_strict {} agents={declared:?}", label(Some(auth)));
        self.calls.lock().unwrap().push(call);
        answer(&self.key_strict)
    }

    async fn access_group_ceiling(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        strict: bool,
    ) -> Result<Option<BTreeSet<String>>, AgentAccessError> {
        let call = format!("ceiling {} strict={strict}", label(auth));
        self.calls.lock().unwrap().push(call);
        answer(&self.ceiling)
    }

    async fn managed_ceilings(
        &self,
        agent_id: &str,
        access_group_ids: &[String],
    ) -> Result<Vec<BTreeSet<String>>, AgentAccessError> {
        let call = format!("managed_ceilings {agent_id} groups={access_group_ids:?}");
        self.calls.lock().unwrap().push(call);
        answer(&self.managed_ceilings)
    }
}

#[derive(Default)]
pub(super) struct FakeTargets {
    pub(super) database: bool,
    // an agent without an entry has no row
    pub(super) agents: BTreeMap<String, Result<Option<AgentResponse>, AgentIdentityFailure>>,
    pub(super) calls: Mutex<Vec<String>>,
}

impl TargetAgents for FakeTargets {
    fn has_database(&self) -> bool {
        self.database
    }

    async fn agent(&self, agent_id: &str) -> Result<Option<AgentResponse>, AgentIdentityFailure> {
        self.calls.lock().unwrap().push(format!("agent {agent_id}"));
        self.agents.get(agent_id).cloned().unwrap_or(Ok(None))
    }
}

pub(super) struct FakeHumans {
    pub(super) reloaded_key: Answer<UserAPIKeyAuth>,
    pub(super) human_failure: Option<u16>,
    // a source without a team is always admitted, a team source only when allowed
    pub(super) sources: Answer<Vec<UserAPIKeyAuth>>,
    // stands in for a source port that ignores allowed_team_ids
    pub(super) admit_every_team: bool,
    // None answers with the auth itself as the one context
    pub(super) contexts: Option<Answer<Vec<UserAPIKeyAuth>>>,
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) context_auths: Mutex<Vec<UserAPIKeyAuth>>,
}

impl Default for FakeHumans {
    fn default() -> Self {
        Self {
            reloaded_key: Err(401),
            human_failure: None,
            sources: Ok(Vec::new()),
            admit_every_team: false,
            contexts: None,
            calls: Mutex::new(Vec::new()),
            context_auths: Mutex::new(Vec::new()),
        }
    }
}

impl HumanSources for FakeHumans {
    async fn reload_admitted_key(
        &self,
        key_hash: &SecretString,
        check_db_only: bool,
    ) -> Result<UserAPIKeyAuth, AgentAccessError> {
        let call = format!(
            "reload_key {} check_db_only={check_db_only}",
            key_hash.expose_secret()
        );
        self.calls.lock().unwrap().push(call);
        answer(&self.reloaded_key)
    }

    async fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<UserAPIKeyAuth, AgentAccessError> {
        let call = format!("reload_user {user_id} fresh={requires_fresh_policy}");
        self.calls.lock().unwrap().push(call);
        let reloaded = self
            .human_failure
            .map_or_else(|| Ok(auth(&format!(r#"{{"user_id":"{user_id}"}}"#))), Err);
        answer(&reloaded)
    }

    async fn admitted_subject_sources(
        &self,
        human: &UserAPIKeyAuth,
        allowed_team_ids: Option<&BTreeSet<String>>,
    ) -> Result<Vec<UserAPIKeyAuth>, AgentAccessError> {
        let call = format!(
            "sources {} allowed={allowed_team_ids:?}",
            label(Some(human))
        );
        self.calls.lock().unwrap().push(call);
        let admitted = |source: &UserAPIKeyAuth| {
            self.admit_every_team
                || source.view.token.team_id.as_ref().is_none_or(|team_id| {
                    allowed_team_ids.is_none_or(|allowed| allowed.contains(team_id))
                })
        };
        Ok(answer(&self.sources)?
            .into_iter()
            .filter(admitted)
            .collect())
    }

    async fn effective_contexts(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> Result<Vec<UserAPIKeyAuth>, AgentAccessError> {
        self.context_auths.lock().unwrap().push(auth.clone());
        self.contexts
            .as_ref()
            .map_or_else(|| Ok(vec![auth.clone()]), answer)
    }
}

pub(super) fn resolver(
    grants: FakeGrants,
    targets: FakeTargets,
    humans: FakeHumans,
) -> FakeResolver {
    AgentAccessResolver {
        registry: Arc::new(AgentRegistry::default()),
        grants,
        targets,
        humans,
    }
}
