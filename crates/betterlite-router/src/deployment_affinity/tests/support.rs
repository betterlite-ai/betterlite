use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_core::api::response_id_codecs::response::build_response_id;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use crate::affinity_claims::AffinityClaims;
use crate::affinity_claims::identity::{
    AffinityFlag, AffinityKey, CACHE_KEY_PREFIX, affinity_cache_key, hash_user_key,
    session_affinity_cache_key,
};
use crate::affinity_claims::local::LocalPins;
use crate::affinity_claims::pin::{AffinityPin, StoredPin};
use crate::deployment_affinity::error::DeploymentAffinityError;
use crate::deployment_affinity::json::PREVIOUS_RESPONSE_ID;
use crate::deployment_affinity::{
    DEFAULT_TTL_SECONDS, DeploymentAffinity, FilterRequest, PreCallRequest, PriorityGroups,
    Settings,
};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_state_store::{
    BreakerClock, RedisSettings, RoutingStateStore, Settings as SharedSettings,
};
use crate::routing_value_store::{Clock, PinValue, RouteKey, RouteValue, StoreRead, StoreSettings};

pub(crate) const MODEL_GROUP: &str = "gpt-4o";

pub(crate) struct Fixture {
    pub(crate) service: DeploymentAffinity,
    now: Arc<Mutex<f64>>,
}

impl Fixture {
    pub(crate) fn new(settings: Settings, priority_groups: &[&str]) -> Self {
        Self::build(settings, priority_groups, None)
    }

    pub(crate) fn with_shared(
        settings: Settings,
        priority_groups: &[&str],
        shared: RoutingStateStore,
    ) -> Self {
        Self::build(settings, priority_groups, Some(shared))
    }

    fn build(
        settings: Settings,
        priority_groups: &[&str],
        shared: Option<RoutingStateStore>,
    ) -> Self {
        let now = Arc::new(Mutex::new(0.0_f64));
        let handle = Arc::clone(&now);
        let clock: Clock = Arc::new(move || *handle.lock().unwrap_or_else(PoisonError::into_inner));
        let local = LocalPins::new(local_settings(), clock).unwrap();
        let groups: Vec<String> = priority_groups
            .iter()
            .map(|group| (*group).to_owned())
            .collect();
        let predicate: PriorityGroups =
            Arc::new(move |model: &str| groups.iter().any(|group| group == model));
        Self {
            service: DeploymentAffinity::new(
                AffinityClaims { local, shared },
                settings,
                Some(predicate),
            ),
            now,
        }
    }

    pub(crate) fn set(&self, seconds: f64) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) = seconds;
    }

    pub(crate) async fn filter(
        &self,
        pool: &[DeploymentSpec],
        kwargs: Option<&JsonObject>,
    ) -> Vec<String> {
        self.filter_at(MODEL_GROUP, pool, kwargs, None).await
    }

    pub(crate) async fn selected(&self, kwargs: &JsonObject) -> Vec<String> {
        self.filter(&pool(&["a", "b"]), Some(kwargs)).await
    }

    pub(crate) async fn filter_at(
        &self,
        model: &str,
        pool: &[DeploymentSpec],
        kwargs: Option<&JsonObject>,
        target_order: Option<i64>,
    ) -> Vec<String> {
        let selected = self
            .service
            .filter_deployments(&FilterRequest {
                model,
                deployments: pool,
                request_kwargs: kwargs,
                target_order,
            })
            .await
            .unwrap();
        ids(&selected)
    }

    pub(crate) async fn filter_error(
        &self,
        pool: &[DeploymentSpec],
        kwargs: Option<&JsonObject>,
    ) -> DeploymentAffinityError {
        self.service
            .filter_deployments(&FilterRequest {
                model: MODEL_GROUP,
                deployments: pool,
                request_kwargs: kwargs,
                target_order: None,
            })
            .await
            .unwrap_err()
    }

    pub(crate) async fn hook(
        &self,
        kwargs: &JsonObject,
        model_info: Option<&JsonObject>,
    ) -> Result<(), DeploymentAffinityError> {
        self.service
            .pre_call_deployment_hook(&PreCallRequest {
                request_kwargs: kwargs,
                model_info,
            })
            .await
    }

    pub(crate) fn seed(&self, key: &AffinityKey, stored: &StoredPin, ttl_seconds: u32) {
        self.service
            .claims
            .local
            .set(key, stored, ttl_seconds)
            .unwrap();
    }

    pub(crate) fn expiry(&self, key: &AffinityKey) -> Option<f64> {
        self.service.claims.local.expiry_seconds(key).unwrap()
    }
}

pub(crate) fn fixture(flags: &[AffinityFlag]) -> Fixture {
    Fixture::new(settings(flags), &[])
}

pub(crate) fn fixture_ttl(flags: &[AffinityFlag], ttl_seconds: u32) -> Fixture {
    Fixture::new(settings_with_ttl(flags, ttl_seconds), &[])
}

pub(crate) fn grouped(defaults: &[AffinityFlag], entries: &[(&str, &[&str])]) -> Fixture {
    Fixture::new(with_groups(settings(defaults), entries), &[])
}

pub(crate) fn settings(flags: &[AffinityFlag]) -> Settings {
    settings_with_ttl(flags, DEFAULT_TTL_SECONDS)
}

pub(crate) fn settings_with_ttl(flags: &[AffinityFlag], ttl_seconds: u32) -> Settings {
    Settings {
        ttl_seconds,
        defaults: flags.iter().copied().collect(),
        model_group_affinity_config: None,
    }
}

pub(crate) fn with_groups(mut settings: Settings, entries: &[(&str, &[&str])]) -> Settings {
    let config = groups(entries);
    settings.model_group_affinity_config = Some(config);
    settings
}

pub(crate) fn groups(entries: &[(&str, &[&str])]) -> IndexMap<String, Vec<String>> {
    let mut groups = IndexMap::with_capacity(entries.len());
    for (group, flags) in entries {
        groups.insert(
            (*group).to_owned(),
            flags.iter().map(|flag| (*flag).to_owned()).collect(),
        );
    }
    groups
}

pub(crate) fn deployment(model_name: &str, id: &str) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(format!("openai/{id}")));
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new(
        model_name.to_owned(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

pub(crate) fn pool(ids: &[&str]) -> Vec<DeploymentSpec> {
    ids.iter().map(|id| deployment(MODEL_GROUP, id)).collect()
}

pub(crate) fn ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    deployments
        .iter()
        .map(|deployment| deployment.id().unwrap().to_owned())
        .collect()
}

pub(crate) fn user_pin(user_key: &str) -> AffinityKey {
    affinity_cache_key(MODEL_GROUP, user_key).unwrap()
}

pub(crate) fn session_pin(session_id: &str) -> AffinityKey {
    session_affinity_cache_key(MODEL_GROUP, session_id, None).unwrap()
}

pub(crate) fn scoped_session_pin(user_key: &str, session_id: &str) -> AffinityKey {
    session_affinity_cache_key(MODEL_GROUP, session_id, Some(user_key)).unwrap()
}

pub(crate) fn pin(model_id: &str) -> StoredPin {
    StoredPin::Pin(AffinityPin(IndexMap::from([(
        "model_id".to_owned(),
        model_id.to_owned(),
    )])))
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn number(value: u64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(crate) fn request(entries: &[(&str, Json)]) -> JsonObject {
    let mut kwargs = JsonObject::with_capacity(entries.len());
    for (key, value) in entries {
        kwargs.insert((*key).to_owned(), value.clone());
    }
    kwargs
}

pub(crate) fn metadata(fields: &[(&str, Json)]) -> JsonObject {
    request(&[("metadata", Json::Object(request(fields)))])
}

pub(crate) fn caller_metadata(user_key: &str) -> JsonObject {
    metadata(&[("user_api_key_hash", text(user_key))])
}

pub(crate) fn session_metadata(session_id: &str) -> JsonObject {
    metadata(&[("session_id", text(session_id))])
}

pub(crate) fn caller_session_metadata(user_key: &str, session_id: &str) -> JsonObject {
    metadata(&[
        ("user_api_key_hash", text(user_key)),
        ("session_id", text(session_id)),
    ])
}

pub(crate) fn marked_session_metadata(session_id: &str, ttl_seconds: u64) -> JsonObject {
    metadata(&[
        ("session_id", text(session_id)),
        ("_session_deployment_affinity_ttl", number(ttl_seconds)),
    ])
}

pub(crate) fn generated_session_metadata(session_id: &str) -> JsonObject {
    metadata(&[
        ("session_id", text(session_id)),
        ("litellm_session_id_generated", Json::Bool(true)),
    ])
}

pub(crate) fn generated_flag_text(session_id: &str) -> JsonObject {
    metadata(&[
        ("session_id", text(session_id)),
        ("litellm_session_id_generated", text("yes")),
    ])
}

pub(crate) fn session_and_response(session_id: &str, model_id: &str) -> JsonObject {
    let mut kwargs = session_metadata(session_id);
    kwargs.insert(
        PREVIOUS_RESPONSE_ID.to_owned(),
        text(&encoded_previous_response_id(model_id)),
    );
    kwargs
}

pub(crate) fn litellm_metadata(fields: &[(&str, Json)]) -> JsonObject {
    request(&[("litellm_metadata", Json::Object(request(fields)))])
}

pub(crate) fn model_info(model_id: &str) -> JsonObject {
    request(&[("id", text(model_id))])
}

pub(crate) fn hook_request(fields: &[(&str, Json)], model_id: Option<&str>) -> JsonObject {
    let mut kwargs = metadata(fields);
    if let Some(id) = model_id {
        kwargs.insert("model_info".to_owned(), Json::Object(model_info(id)));
    }
    kwargs
}

pub(crate) fn hook_kwargs(identity: (&str, &str), model_id: &str) -> JsonObject {
    let (field, value) = identity;
    hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            (field, text(value)),
        ],
        Some(model_id),
    )
}

pub(crate) fn marked_hook(session_id: &str, marker_ttl: u64, model_id: &str) -> JsonObject {
    hook_request(
        &[
            ("deployment_model_name", text(MODEL_GROUP)),
            ("session_id", text(session_id)),
            ("_session_deployment_affinity_ttl", number(marker_ttl)),
        ],
        Some(model_id),
    )
}

pub(crate) fn litellm_hook(identity: (&str, &str), model_id: &str) -> JsonObject {
    let (field, value) = identity;
    let mut fields = request(&[("deployment_model_name", text(MODEL_GROUP))]);
    fields.insert(field.to_owned(), text(value));
    fields.insert("model_info".to_owned(), Json::Object(model_info(model_id)));
    request(&[("litellm_metadata", Json::Object(fields))])
}

pub(crate) fn encoded_previous_response_id(model_id: &str) -> String {
    build_response_id(Some("openai"), Some(model_id), "resp_1")
}

const fn local_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: Some(1024),
        loaded_max_size_per_item_kib: 1024,
    }
}

// the shared tier a running proxy has: a redis client behind the state store
pub(crate) fn shared_store(server: &FakeRedis) -> RoutingStateStore {
    let settings = SharedSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.port())),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 0.25,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(60),
            timeout_min_duration: Duration::from_secs(5),
            timeout_log_interval: Duration::from_secs(1),
        },
    };
    let clock: BreakerClock = Arc::new(|| Duration::ZERO);
    RoutingStateStore::new(&settings, clock).unwrap()
}

// claim_affinity_pin's redis script writes the serialized pin into the route value's model_id
pub(crate) fn pin_value(stored: &StoredPin) -> RouteValue {
    RouteValue::Pin(PinValue {
        model_id: stored.wire_text(),
    })
}

pub(crate) async fn seed_shared_value(
    store: &RoutingStateStore,
    key: &AffinityKey,
    value: RouteValue,
    ttl_seconds: u32,
) {
    let written = store
        .compare_and_set(
            &key.0,
            &StoreRead::Missing,
            &value,
            Some(f64::from(ttl_seconds)),
        )
        .await
        .unwrap();
    assert!(written.value.swapped);
}

// a pin seeded under a literal raw key past the route key cap, read through a fixture with the shared tier
pub(crate) async fn seeded_filter(
    raw: String,
    flags: &[AffinityFlag],
    model_group: &str,
    kwargs: &JsonObject,
) -> Vec<String> {
    let server = FakeRedis::start().await;
    let store = shared_store(&server);
    let key = AffinityKey(RouteKey(raw));
    seed_shared_value(&store, &key, pin_value(&pin("a")), 600).await;
    let fixture = Fixture::with_shared(settings(flags), &[], store);
    let healthy = vec![deployment(model_group, "a"), deployment(model_group, "b")];
    fixture
        .filter_at(model_group, &healthy, Some(kwargs), None)
        .await
}

// a caller key past the route key cap, seeded under the literal an uncapped builder would produce
pub(crate) async fn filtered_with_long_caller_key(
    model_group: &str,
    user_key: &str,
) -> Vec<String> {
    let raw = format!(
        "{CACHE_KEY_PREFIX}:{model_group}:{}",
        hash_user_key(user_key)
    );
    seeded_filter(
        raw,
        &[AffinityFlag::Deployment],
        model_group,
        &caller_metadata(user_key),
    )
    .await
}

// a session key past the route key cap, seeded under the literal an uncapped builder would produce
pub(crate) async fn filtered_with_long_session_key(session_id: &str) -> Vec<String> {
    let raw = format!("{CACHE_KEY_PREFIX}:session:{MODEL_GROUP}:unscoped:{session_id}");
    seeded_filter(
        raw,
        &[AffinityFlag::Session],
        MODEL_GROUP,
        &session_metadata(session_id),
    )
    .await
}
