#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard};

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
        ("end_user_max_budget".to_owned(), decimal_zero()),
        ("last_refreshed_at".to_owned(), decimal_zero()),
        ("max_budget".to_owned(), decimal_zero()),
        ("organization_max_budget".to_owned(), decimal_zero()),
        ("soft_budget".to_owned(), decimal_zero()),
        ("team_max_budget".to_owned(), decimal_zero()),
        ("team_member_spend".to_owned(), decimal_zero()),
        ("team_soft_budget".to_owned(), decimal_zero()),
        ("team_spend".to_owned(), decimal_zero()),
        ("user_max_budget".to_owned(), decimal_zero()),
        ("user_spend".to_owned(), decimal_zero()),
    ]);
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the test key auth")
}

// a cached key only decodes when its flattened optional decimals are present, since null fails the view
fn decimal_zero() -> JsonData {
    JsonData::Number(serde_json::Number::from(0_i64))
}

fn groups_value(groups: Option<Vec<String>>) -> JsonData {
    groups.map_or(JsonData::Null, |ids| {
        JsonData::Array(ids.into_iter().map(JsonData::String).collect())
    })
}

fn groups_json(groups: Option<Vec<String>>) -> String {
    let Some(ids) = groups else {
        return "null".to_owned();
    };
    let listed: Vec<String> = ids.iter().map(|id| format!("\"{id}\"")).collect();
    format!("[{}]", listed.join(","))
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

#[derive(Clone)]
pub(super) enum CachedFixture {
    Team(Box<LiteLLMTeamTableCachedObj>),
    Key(Box<UserAPIKeyAuth>),
}

impl CachedFixture {
    fn cache_key(&self, id: &str) -> String {
        match self {
            Self::Team(_) => format!("team_id:{id}"),
            Self::Key(_) => id.to_owned(),
        }
    }

    fn into_team(self) -> Option<LiteLLMTeamTableCachedObj> {
        match self {
            Self::Team(team) => Some(*team),
            Self::Key(_) => None,
        }
    }

    fn into_key(self) -> Option<UserAPIKeyAuth> {
        match self {
            Self::Key(auth) => Some(*auth),
            Self::Team(_) => None,
        }
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().expect("the scripted fixture")
}

#[derive(Default)]
pub(super) struct ScriptedCaches {
    entries: Mutex<BTreeMap<String, CachedFixture>>,
    reads: Mutex<Vec<String>>,
    writes: Mutex<Vec<(String, CachedFixture)>>,
    failing: Mutex<BTreeSet<String>>,
}

impl ScriptedCaches {
    pub(super) fn seed_team(&self, team_id: &str, team: LiteLLMTeamTableCachedObj) {
        self.put(
            format!("team_id:{team_id}"),
            CachedFixture::Team(Box::new(team)),
        );
    }

    pub(super) fn seed_key(&self, token: &str, auth: UserAPIKeyAuth) {
        self.put(token.to_owned(), CachedFixture::Key(Box::new(auth)));
    }

    pub(super) fn failing(&self, ids: &[&str]) {
        let mut failing = locked(&self.failing);
        failing.extend(ids.iter().map(|id| String::from(*id)));
    }

    pub(super) fn reads(&self) -> Vec<String> {
        locked(&self.reads).clone()
    }

    pub(super) fn written(&self) -> Vec<String> {
        locked(&self.writes)
            .iter()
            .map(|(id, _entry)| id.clone())
            .collect()
    }

    pub(super) fn stored_team(&self, team_id: &str) -> Option<LiteLLMTeamTableCachedObj> {
        self.entry(&format!("team_id:{team_id}"))
            .and_then(CachedFixture::into_team)
    }

    pub(super) fn stored_key(&self, token: &str) -> Option<UserAPIKeyAuth> {
        self.entry(token).and_then(CachedFixture::into_key)
    }

    fn put(&self, key: String, entry: CachedFixture) {
        locked(&self.entries).insert(key, entry);
    }

    fn entry(&self, key: &str) -> Option<CachedFixture> {
        let entries = locked(&self.entries);
        entries.get(key).cloned()
    }

    fn record_read(&self, key: &str) {
        locked(&self.reads).push(key.to_owned());
    }

    fn store_entry(&self, id: &str, entry: CachedFixture) -> Result<(), MemberEffectsError> {
        if locked(&self.failing).contains(id) {
            return Err(store_failure());
        }
        locked(&self.writes).push((id.to_owned(), entry.clone()));
        self.put(entry.cache_key(id), entry);
        Ok(())
    }
}

impl MemberEffectCaches for ScriptedCaches {
    fn read_team(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<LiteLLMTeamTableCachedObj>, MemberEffectsError>> + Send
    {
        self.record_read(key);
        ready(Ok(self.entry(key).and_then(CachedFixture::into_team)))
    }

    fn store_team(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> impl Future<Output = Result<(), MemberEffectsError>> + Send {
        ready(self.store_entry(team_id, CachedFixture::Team(Box::new(team.clone()))))
    }

    fn read_key(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<Option<UserAPIKeyAuth>, MemberEffectsError>> + Send {
        self.record_read(token);
        ready(Ok(self.entry(token).and_then(CachedFixture::into_key)))
    }

    fn store_key(
        &self,
        token: &str,
        auth: &mut UserAPIKeyAuth,
    ) -> impl Future<Output = Result<(), MemberEffectsError>> + Send {
        ready(self.store_entry(token, CachedFixture::Key(Box::new(auth.clone()))))
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
        locked(&self.agents).insert(agent.agent_name.clone(), agent);
    }

    pub(super) fn rejecting(&self, agent_name: &str) {
        *locked(&self.reject) = Some(agent_name.to_owned());
    }

    pub(super) fn ops(&self) -> Vec<RegistryOp> {
        locked(&self.ops).clone()
    }

    pub(super) fn agent(&self, agent_name: &str) -> Option<AgentResponse> {
        let agents = locked(&self.agents);
        agents.get(agent_name).cloned()
    }
}

impl AgentRegistryEffects for ScriptedRegistry {
    fn find(&self, agent_id: &str) -> Option<AgentResponse> {
        locked(&self.ops).push(RegistryOp::Find(agent_id.to_owned()));
        let agents = locked(&self.agents);
        agents
            .values()
            .find(|agent| agent.agent_id == agent_id)
            .cloned()
    }

    fn deregister(&self, agent_name: &str) {
        locked(&self.ops).push(RegistryOp::Deregister(agent_name.to_owned()));
        locked(&self.agents).remove(agent_name);
    }

    fn register(&self, agent: AgentResponse) -> Result<(), RegistrationRejected> {
        locked(&self.ops).push(RegistryOp::Register {
            agent_id: agent.agent_id.clone(),
            agent_name: agent.agent_name.clone(),
            groups: agent.access_group_ids.clone(),
        });
        let rejected = locked(&self.reject)
            .as_deref()
            .is_some_and(|name| name == agent.agent_name.as_str());
        if rejected {
            return Err(RegistrationRejected);
        }
        locked(&self.agents).insert(agent.agent_name.clone(), agent);
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
