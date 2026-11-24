#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::auth_invalidation::clock::SystemClock;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_auth::auth_invalidation::ports::{AuthInvalidationCache, SpendCounterCache};
use betterlite_auth::auth_invalidation::transport::InvalidationTransport;
use betterlite_auth::auth_pubsub::eviction::LocalEviction;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::access_group_member_effects::agents::AgentRegistryEffects;
use crate::access_group_member_effects::cache::{GatewayMemberCaches, MemberEffectCaches};
use crate::access_group_member_effects::error::{MemberEffectsError, RegistrationRejected};
use crate::management_effects::fixtures::auth_cache::memory_auth_cache;

pub(super) fn team_object(team_id: &str, groups: Option<Vec<String>>) -> LiteLLMTeamTableCachedObj {
    let text = format!(
        r#"{{"team_id": "{team_id}", "team_alias": "alias-{team_id}", "tpm_limit": 7, "models": ["gpt-4o"], "access_group_ids": {}}}"#,
        groups_json(groups)
    );
    serde_json::from_str(&text).expect("the test team object")
}

pub(super) fn key_auth(token: &str, groups: Option<Vec<String>>) -> UserAPIKeyAuth {
    let fields = BTreeMap::from([
        ("token".to_owned(), JsonData::String(token.to_owned())),
        ("user_id".to_owned(), JsonData::String("user-1".to_owned())),
        (
            "key_alias".to_owned(),
            JsonData::String("key-one".to_owned()),
        ),
        ("team_id".to_owned(), JsonData::String("team-1".to_owned())),
        ("access_group_ids".to_owned(), groups_value(groups)),
    ]);
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the test key auth")
}

pub(super) fn agent(
    agent_id: &str,
    agent_name: &str,
    groups: Option<Vec<String>>,
) -> AgentResponse {
    let text = format!(
        r#"{{"agent_id": "{agent_id}", "agent_name": "{agent_name}", "agent_card_params": {{}}, "access_group_ids": {}}}"#,
        groups_json(groups)
    );
    serde_json::from_str(&text).expect("the test agent")
}

fn groups_value(groups: Option<Vec<String>>) -> JsonData {
    match groups {
        Some(groups) => JsonData::Array(groups.into_iter().map(JsonData::String).collect()),
        None => JsonData::Null,
    }
}

fn groups_json(groups: Option<Vec<String>>) -> String {
    let Some(groups) = groups else {
        return "null".to_owned();
    };
    let listed: Vec<String> = groups.iter().map(|id| format!("\"{id}\"")).collect();
    format!("[{}]", listed.join(","))
}

#[derive(Default)]
pub(super) struct ScriptedCaches {
    teams: Mutex<BTreeMap<String, LiteLLMTeamTableCachedObj>>,
    keys: Mutex<BTreeMap<String, UserAPIKeyAuth>>,
    reads: Mutex<Vec<String>>,
    team_writes: Mutex<Vec<(String, LiteLLMTeamTableCachedObj)>>,
    key_writes: Mutex<Vec<(String, UserAPIKeyAuth)>>,
    failing: Mutex<BTreeSet<String>>,
}

impl ScriptedCaches {
    pub(super) fn seed_team(&self, team_id: &str, team: LiteLLMTeamTableCachedObj) {
        self.teams
            .lock()
            .expect("the scripted teams")
            .insert(format!("team_id:{team_id}"), team);
    }

    pub(super) fn seed_key(&self, token: &str, auth: UserAPIKeyAuth) {
        self.keys
            .lock()
            .expect("the scripted keys")
            .insert(token.to_owned(), auth);
    }

    pub(super) fn failing(&self, ids: &[&str]) {
        let mut failing = self.failing.lock().expect("the failing stores");
        failing.extend(ids.iter().map(|id| String::from(*id)));
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("the recorded reads").clone()
    }

    pub(super) fn team_writes(&self) -> Vec<(String, LiteLLMTeamTableCachedObj)> {
        self.team_writes
            .lock()
            .expect("the recorded team writes")
            .clone()
    }

    pub(super) fn key_writes(&self) -> Vec<(String, UserAPIKeyAuth)> {
        self.key_writes
            .lock()
            .expect("the recorded key writes")
            .clone()
    }
}

impl MemberEffectCaches for ScriptedCaches {
    async fn read_team(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, MemberEffectsError> {
        self.reads
            .lock()
            .expect("the recorded reads")
            .push(key.to_owned());
        let teams = self.teams.lock().expect("the scripted teams");
        Ok(teams.get(key).cloned())
    }

    async fn store_team(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), MemberEffectsError> {
        if self
            .failing
            .lock()
            .expect("the failing stores")
            .contains(team_id)
        {
            return Err(store_failure());
        }
        self.team_writes
            .lock()
            .expect("the recorded team writes")
            .push((team_id.to_owned(), team.clone()));
        self.teams
            .lock()
            .expect("the scripted teams")
            .insert(format!("team_id:{team_id}"), team.clone());
        Ok(())
    }

    async fn read_key(&self, token: &str) -> Result<Option<UserAPIKeyAuth>, MemberEffectsError> {
        self.reads
            .lock()
            .expect("the recorded reads")
            .push(token.to_owned());
        let keys = self.keys.lock().expect("the scripted keys");
        Ok(keys.get(token).cloned())
    }

    async fn store_key(
        &self,
        token: &str,
        auth: &mut UserAPIKeyAuth,
    ) -> Result<(), MemberEffectsError> {
        if self
            .failing
            .lock()
            .expect("the failing stores")
            .contains(token)
        {
            return Err(store_failure());
        }
        self.key_writes
            .lock()
            .expect("the recorded key writes")
            .push((token.to_owned(), auth.clone()));
        self.keys
            .lock()
            .expect("the scripted keys")
            .insert(token.to_owned(), auth.clone());
        Ok(())
    }
}

pub(super) fn store_failure() -> MemberEffectsError {
    MemberEffectsError::CacheWrite(InvalidationError::SpendOutOfRange(Decimal::ZERO))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RegistryOp {
    Find(String),
    Deregister(String),
    Register {
        agent_id: String,
        agent_name: String,
        groups: Option<Vec<String>>,
    },
}

#[derive(Default)]
pub(super) struct ScriptedRegistry {
    agents: Mutex<BTreeMap<String, AgentResponse>>,
    ops: Mutex<Vec<RegistryOp>>,
    reject: Mutex<Option<String>>,
}

impl ScriptedRegistry {
    pub(super) fn seed(&self, agent: AgentResponse) {
        self.agents
            .lock()
            .expect("the scripted agents")
            .insert(agent.agent_name.clone(), agent);
    }

    pub(super) fn rejecting(&self, agent_name: &str) {
        *self.reject.lock().expect("the rejected agent") = Some(agent_name.to_owned());
    }

    pub(super) fn ops(&self) -> Vec<RegistryOp> {
        self.ops.lock().expect("the recorded operations").clone()
    }

    pub(super) fn agent(&self, agent_name: &str) -> Option<AgentResponse> {
        let agents = self.agents.lock().expect("the scripted agents");
        agents.get(agent_name).cloned()
    }
}

impl AgentRegistryEffects for ScriptedRegistry {
    fn find(&self, agent_id: &str) -> Option<AgentResponse> {
        self.ops
            .lock()
            .expect("the recorded operations")
            .push(RegistryOp::Find(agent_id.to_owned()));
        let agents = self.agents.lock().expect("the scripted agents");
        agents
            .values()
            .find(|agent| agent.agent_id == agent_id)
            .cloned()
    }

    fn deregister(&self, agent_name: &str) {
        self.ops
            .lock()
            .expect("the recorded operations")
            .push(RegistryOp::Deregister(agent_name.to_owned()));
        self.agents
            .lock()
            .expect("the scripted agents")
            .remove(agent_name);
    }

    fn register(&self, agent: AgentResponse) -> Result<(), RegistrationRejected> {
        self.ops
            .lock()
            .expect("the recorded operations")
            .push(RegistryOp::Register {
                agent_id: agent.agent_id.clone(),
                agent_name: agent.agent_name.clone(),
                groups: agent.access_group_ids.clone(),
            });
        let rejected = self
            .reject
            .lock()
            .expect("the rejected agent")
            .as_deref()
            .is_some_and(|name| name == agent.agent_name.as_str());
        if rejected {
            return Err(RegistrationRejected);
        }
        self.agents
            .lock()
            .expect("the scripted agents")
            .insert(agent.agent_name.clone(), agent);
        Ok(())
    }
}

#[derive(Deserialize)]
pub(super) struct KeyView {
    #[serde(default)]
    pub(super) access_group_ids: Option<Vec<String>>,
    #[serde(default)]
    pub(super) user_id: Option<String>,
    #[serde(default)]
    pub(super) key_alias: Option<String>,
}

pub(super) fn key_view(auth: &UserAPIKeyAuth) -> KeyView {
    serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .expect("the cached key view")
}

struct Silent;

#[async_trait]
impl InvalidationTransport for Silent {
    async fn publish(&self, _cache_key: &str, _new_value: Option<f64>, _ttl: Option<f64>) {}

    async fn evict_and_broadcast(&self, _cache_keys: &[String], _local: &dyn LocalEviction) {}
}

fn memory() -> Arc<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 100.0_f64);
    Arc::new(LocalMemoryOps::new(1024, clock).expect("the fixture memory"))
}

const fn dual_settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

pub(super) fn gateway() -> GatewayMemberCaches<DualCache<LocalMemoryOps>> {
    let cache = Arc::new(memory_auth_cache());
    let auth: Arc<dyn AuthInvalidationCache> =
        Arc::<AuthCache<DualCache<LocalMemoryOps>>>::clone(&cache);
    let spend: Arc<dyn SpendCounterCache> =
        Arc::new(DualCache::new(memory(), None, dual_settings()));
    let invalidation = AuthInvalidation {
        auth,
        usage: None,
        spend,
        transport: Arc::new(Silent),
        clock: Arc::new(SystemClock),
        access_group_ttl: 60.0_f64,
    };
    GatewayMemberCaches {
        cache,
        invalidation: Arc::new(invalidation),
    }
}
