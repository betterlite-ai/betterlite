#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::sensitive_data_routing::Settings;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde_json::json;

use super::error::SensitiveRoutingError;
use super::identity::PrincipalIdentity;
use super::ports::SensitiveRoutingCache;
use super::{SensitiveDataRoutingHandler, cache_key, resolve_tenant};

#[derive(Clone, Debug, thiserror::Error)]
#[error("redis is unavailable")]
struct FakeError;

#[derive(Debug, PartialEq, Eq)]
enum CacheCall {
    RedisGet(String),
    RedisTtl(String),
    RedisSet {
        key: String,
        value: String,
        ttl: Duration,
    },
    MemoryGet(String),
    MemorySet {
        key: String,
        value: String,
        ttl: Duration,
    },
}

#[derive(Default)]
struct FakeState {
    redis_configured: bool,
    redis_get: Option<Result<Option<String>, FakeError>>,
    redis_ttl: Option<Result<Option<Duration>, FakeError>>,
    redis_set: Option<Result<(), FakeError>>,
    memory_value: Option<String>,
    calls: Vec<CacheCall>,
}

#[derive(Clone)]
struct FakeCache {
    shared: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl SensitiveRoutingCache for FakeCache {
    type Error = FakeError;

    fn redis_configured(&self) -> bool {
        self.shared.lock().unwrap().redis_configured
    }

    async fn redis_get(&self, key: &str) -> Result<Option<String>, FakeError> {
        let mut state = self.shared.lock().unwrap();
        state.calls.push(CacheCall::RedisGet(key.to_owned()));
        state.redis_get.clone().unwrap_or(Ok(None))
    }

    async fn redis_ttl(&self, key: &str) -> Result<Option<Duration>, FakeError> {
        let mut state = self.shared.lock().unwrap();
        state.calls.push(CacheCall::RedisTtl(key.to_owned()));
        state.redis_ttl.clone().unwrap_or(Ok(None))
    }

    async fn redis_set(&self, key: &str, value: &str, ttl: Duration) -> Result<(), FakeError> {
        let mut state = self.shared.lock().unwrap();
        state.calls.push(CacheCall::RedisSet {
            key: key.to_owned(),
            value: value.to_owned(),
            ttl,
        });
        state.redis_set.clone().unwrap_or(Ok(()))
    }

    async fn memory_get(&self, key: &str) -> Option<String> {
        let mut state = self.shared.lock().unwrap();
        state.calls.push(CacheCall::MemoryGet(key.to_owned()));
        state.memory_value.clone()
    }

    async fn memory_set(&self, key: &str, value: &str, ttl: Duration) {
        let mut state = self.shared.lock().unwrap();
        state.calls.push(CacheCall::MemorySet {
            key: key.to_owned(),
            value: value.to_owned(),
            ttl,
        });
        state.memory_value = Some(value.to_owned());
    }
}

const fn state(
    redis_configured: bool,
    redis_get: Option<Result<Option<String>, FakeError>>,
    redis_ttl: Option<Result<Option<Duration>, FakeError>>,
    redis_set: Option<Result<(), FakeError>>,
    memory_value: Option<String>,
) -> FakeState {
    FakeState {
        redis_configured,
        redis_get,
        redis_ttl,
        redis_set,
        memory_value,
        calls: Vec::new(),
    }
}

fn memory_only(model: &str) -> FakeState {
    state(false, None, None, None, Some(model.to_owned()))
}

fn redis_hit(model: &str, remaining: Option<Duration>) -> FakeState {
    state(
        true,
        Some(Ok(Some(model.to_owned()))),
        Some(Ok(remaining)),
        None,
        None,
    )
}

fn redis_miss(memory: Option<&str>) -> FakeState {
    state(true, Some(Ok(None)), None, None, memory.map(str::to_owned))
}

fn redis_get_failure(memory: &str) -> FakeState {
    state(
        true,
        Some(Err(FakeError)),
        None,
        None,
        Some(memory.to_owned()),
    )
}

fn redis_ttl_failure(memory: &str) -> FakeState {
    state(
        true,
        Some(Ok(Some("gpt-4o-sensitive".to_owned()))),
        Some(Err(FakeError)),
        None,
        Some(memory.to_owned()),
    )
}

const fn redis_set_outcome(outcome: Result<(), FakeError>) -> FakeState {
    state(true, None, None, Some(outcome), None)
}

fn handler(
    state: FakeState,
    ttl_seconds: u64,
) -> (
    SensitiveDataRoutingHandler<FakeCache>,
    Arc<Mutex<FakeState>>,
) {
    let shared = Arc::new(Mutex::new(state));
    let cache = FakeCache {
        shared: Arc::clone(&shared),
    };
    let settings = Settings { ttl_seconds };
    (SensitiveDataRoutingHandler::new(cache, settings), shared)
}

fn calls(shared: &Arc<Mutex<FakeState>>) -> Vec<CacheCall> {
    shared.lock().unwrap().calls.clone()
}

fn request(model: Option<&str>) -> JsonMap {
    let mut data = JsonMap::new();
    if let Some(model) = model {
        data.insert("model".to_owned(), json!(model));
    }
    data
}

const fn keyed(api_key: &str) -> PrincipalIdentity<'_> {
    PrincipalIdentity {
        hashed_api_key: Some(api_key),
        user_id: None,
        team_id: None,
        org_id: None,
    }
}

const fn claims<'identity>(
    user_id: Option<&'identity str>,
    team_id: Option<&'identity str>,
    org_id: Option<&'identity str>,
) -> PrincipalIdentity<'identity> {
    PrincipalIdentity {
        hashed_api_key: None,
        user_id,
        team_id,
        org_id,
    }
}

#[test]
fn cache_key_layout_matches_litellm() {
    assert_eq!(
        cache_key("session-1", "key-hash"),
        "{sensitive_route:key-hash:session-1}:model"
    );
}

#[test]
fn tenant_prefers_the_hashed_api_key() {
    let principal = PrincipalIdentity {
        hashed_api_key: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        org_id: Some("org-1"),
    };
    assert_eq!(resolve_tenant(principal), "hashed-key");
}

#[test]
fn tenant_joins_user_team_and_org_in_order() {
    let principal = claims(Some("user-1"), Some("team-1"), Some("org-1"));
    assert_eq!(
        resolve_tenant(principal),
        "user:user-1|team:team-1|org:org-1"
    );
}

#[test]
fn tenant_skips_missing_claims() {
    assert_eq!(
        resolve_tenant(claims(None, Some("team-1"), None)),
        "team:team-1"
    );
    assert_eq!(
        resolve_tenant(claims(Some("user-1"), None, None)),
        "user:user-1"
    );
    assert_eq!(
        resolve_tenant(claims(None, None, Some("org-1"))),
        "org:org-1"
    );
}

#[test]
fn tenant_skips_empty_claims() {
    assert_eq!(
        resolve_tenant(claims(Some(""), Some("team-1"), Some(""))),
        "team:team-1"
    );
}

#[test]
fn tenant_falls_back_to_default_without_an_identity() {
    assert_eq!(resolve_tenant(claims(None, None, None)), "default");
    assert_eq!(
        resolve_tenant(claims(Some(""), Some(""), Some(""))),
        "default"
    );
}

#[test]
fn tenant_ignores_an_empty_api_key() {
    let principal = PrincipalIdentity {
        hashed_api_key: Some(""),
        user_id: Some("user-1"),
        team_id: None,
        org_id: None,
    };
    assert_eq!(resolve_tenant(principal), "user:user-1");
}

#[test]
fn handler_takes_its_ttl_from_the_config_settings() {
    let shared = Arc::new(Mutex::new(FakeState::default()));
    let cache = FakeCache { shared };
    let handler = SensitiveDataRoutingHandler::new(cache, Settings::default());
    assert_eq!(handler.ttl_seconds, 3600);
}

#[tokio::test]
async fn applies_a_redis_routing_override() {
    let (handler, shared) = handler(
        redis_hit("gpt-4o-sensitive", Some(Duration::from_secs(120))),
        900,
    );
    let mut data = request(Some("gpt-4o"));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(data.get("model"), Some(&json!("gpt-4o-sensitive")));
    assert_eq!(
        data.get("metadata"),
        Some(&json!({
            "sensitive_data_routing_applied": true,
            "sensitive_data_routing_original_model": "gpt-4o",
        }))
    );
    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(
        calls(&shared),
        vec![
            CacheCall::RedisGet(key.clone()),
            CacheCall::RedisTtl(key.clone()),
            CacheCall::MemorySet {
                key,
                value: "gpt-4o-sensitive".to_owned(),
                ttl: Duration::from_secs(120),
            },
        ]
    );
}

#[tokio::test]
async fn applies_a_memory_routing_override_when_redis_is_absent() {
    let (handler, shared) = handler(memory_only("gpt-4o-sensitive"), 900);
    let mut data = request(Some("gpt-4o"));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(data.get("model"), Some(&json!("gpt-4o-sensitive")));
    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(calls(&shared), vec![CacheCall::MemoryGet(key)]);
}

#[tokio::test]
async fn reads_memory_when_redis_has_no_routing() {
    let (handler, shared) = handler(redis_miss(Some("gpt-4o-cheap")), 900);
    let mut data = request(Some("gpt-4o"));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(data.get("model"), Some(&json!("gpt-4o-cheap")));
    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(
        calls(&shared),
        vec![CacheCall::RedisGet(key.clone()), CacheCall::MemoryGet(key)]
    );
}

#[tokio::test]
async fn falls_back_to_memory_when_the_redis_read_fails() {
    let (handler, shared) = handler(redis_get_failure("gpt-4o-cheap"), 900);
    let mut data = request(Some("gpt-4o"));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(data.get("model"), Some(&json!("gpt-4o-cheap")));
    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(
        calls(&shared),
        vec![CacheCall::RedisGet(key.clone()), CacheCall::MemoryGet(key)]
    );
}

#[tokio::test]
async fn falls_back_to_memory_when_the_redis_ttl_read_fails() {
    let (handler, shared) = handler(redis_ttl_failure("gpt-4o-cheap"), 900);
    let mut data = request(Some("gpt-4o"));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(data.get("model"), Some(&json!("gpt-4o-cheap")));
    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(
        calls(&shared),
        vec![
            CacheCall::RedisGet(key.clone()),
            CacheCall::RedisTtl(key.clone()),
            CacheCall::MemoryGet(key),
        ]
    );
}

#[tokio::test]
async fn uses_the_configured_ttl_when_redis_has_none_left() {
    let (handler, shared) = handler(redis_hit("gpt-4o-sensitive", None), 900);
    let mut data = request(Some("gpt-4o"));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(
        calls(&shared),
        vec![
            CacheCall::RedisGet(key.clone()),
            CacheCall::RedisTtl(key.clone()),
            CacheCall::MemorySet {
                key,
                value: "gpt-4o-sensitive".to_owned(),
                ttl: Duration::from_secs(900),
            },
        ]
    );
}

#[tokio::test]
async fn leaves_the_request_alone_without_a_session_id() {
    let (handler, shared) = handler(redis_hit("gpt-4o-sensitive", None), 900);
    let mut data = request(Some("gpt-4o"));

    handler
        .async_pre_call_hook(&mut data, None, keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(data, request(Some("gpt-4o")));
    assert!(calls(&shared).is_empty());
}

#[tokio::test]
async fn leaves_the_request_alone_when_no_routing_exists() {
    let (handler, _shared) = handler(redis_miss(None), 900);
    let mut data = request(Some("gpt-4o"));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(data, request(Some("gpt-4o")));
}

#[tokio::test]
async fn leaves_the_request_alone_when_it_already_targets_the_routed_model() {
    let (handler, shared) = handler(redis_hit("gpt-4o", None), 900);
    let mut data = request(Some("gpt-4o"));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(data, request(Some("gpt-4o")));
    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(
        calls(&shared),
        vec![
            CacheCall::RedisGet(key.clone()),
            CacheCall::RedisTtl(key.clone()),
            CacheCall::MemorySet {
                key,
                value: "gpt-4o".to_owned(),
                ttl: Duration::from_secs(900),
            },
        ]
    );
}

#[tokio::test]
async fn records_an_absent_model_as_null() {
    let (handler, _shared) = handler(memory_only("gpt-4o-sensitive"), 900);
    let mut data = request(None);

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(data.get("model"), Some(&json!("gpt-4o-sensitive")));
    assert_eq!(
        data.get("metadata"),
        Some(&json!({
            "sensitive_data_routing_applied": true,
            "sensitive_data_routing_original_model": null,
        }))
    );
}

#[tokio::test]
async fn keeps_existing_metadata_fields() {
    let (handler, _shared) = handler(memory_only("gpt-4o-sensitive"), 900);
    let mut data = request(Some("gpt-4o"));
    data.insert("metadata".to_owned(), json!({"trace_id": "trace-1"}));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(
        data.get("metadata"),
        Some(&json!({
            "trace_id": "trace-1",
            "sensitive_data_routing_applied": true,
            "sensitive_data_routing_original_model": "gpt-4o",
        }))
    );
}

#[tokio::test]
async fn replaces_null_metadata_with_an_object() {
    let (handler, _shared) = handler(memory_only("gpt-4o-sensitive"), 900);
    let mut data = request(Some("gpt-4o"));
    data.insert("metadata".to_owned(), json!(null));

    handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap();

    assert_eq!(
        data.get("metadata"),
        Some(&json!({
            "sensitive_data_routing_applied": true,
            "sensitive_data_routing_original_model": "gpt-4o",
        }))
    );
}

#[tokio::test]
async fn rejects_metadata_that_is_not_an_object() {
    let (handler, _shared) = handler(memory_only("gpt-4o-sensitive"), 900);
    let mut data = request(Some("gpt-4o"));
    data.insert("metadata".to_owned(), json!("not-an-object"));

    let error = handler
        .async_pre_call_hook(&mut data, Some("session-1"), keyed("hashed-key"))
        .await
        .unwrap_err();

    assert!(matches!(error, SensitiveRoutingError::MetadataNotObject));
    assert_eq!(data.get("model"), Some(&json!("gpt-4o")));
    assert_eq!(data.get("metadata"), Some(&json!("not-an-object")));
}

#[tokio::test]
async fn set_session_routing_writes_redis_before_memory() {
    let (handler, shared) = handler(redis_set_outcome(Ok(())), 900);

    handler
        .set_session_routing(
            "session-1",
            "gpt-4o-sensitive",
            keyed("hashed-key"),
            Some("pii-guard"),
        )
        .await;

    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(
        calls(&shared),
        vec![
            CacheCall::RedisSet {
                key: key.clone(),
                value: "gpt-4o-sensitive".to_owned(),
                ttl: Duration::from_secs(900),
            },
            CacheCall::MemorySet {
                key,
                value: "gpt-4o-sensitive".to_owned(),
                ttl: Duration::from_secs(900),
            },
        ]
    );
}

#[tokio::test]
async fn set_session_routing_still_writes_memory_when_redis_fails() {
    let (handler, shared) = handler(redis_set_outcome(Err(FakeError)), 900);

    handler
        .set_session_routing("session-1", "gpt-4o-sensitive", keyed("hashed-key"), None)
        .await;

    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(
        calls(&shared),
        vec![
            CacheCall::RedisSet {
                key: key.clone(),
                value: "gpt-4o-sensitive".to_owned(),
                ttl: Duration::from_secs(900),
            },
            CacheCall::MemorySet {
                key,
                value: "gpt-4o-sensitive".to_owned(),
                ttl: Duration::from_secs(900),
            },
        ]
    );
}

#[tokio::test]
async fn set_session_routing_skips_redis_when_it_is_not_configured() {
    let (handler, shared) = handler(FakeState::default(), 900);

    handler
        .set_session_routing("session-1", "gpt-4o-sensitive", keyed("hashed-key"), None)
        .await;

    let key = "{sensitive_route:hashed-key:session-1}:model".to_owned();
    assert_eq!(
        calls(&shared),
        vec![CacheCall::MemorySet {
            key,
            value: "gpt-4o-sensitive".to_owned(),
            ttl: Duration::from_secs(900),
        }]
    );
}

#[tokio::test]
async fn set_session_routing_scopes_the_key_to_the_claims() {
    let (handler, shared) = handler(FakeState::default(), 900);

    handler
        .set_session_routing(
            "session-1",
            "gpt-4o-sensitive",
            claims(Some("user-1"), None, Some("org-1")),
            None,
        )
        .await;

    assert_eq!(
        calls(&shared),
        vec![CacheCall::MemorySet {
            key: "{sensitive_route:user:user-1|org:org-1:session-1}:model".to_owned(),
            value: "gpt-4o-sensitive".to_owned(),
            ttl: Duration::from_secs(900),
        }]
    );
}
