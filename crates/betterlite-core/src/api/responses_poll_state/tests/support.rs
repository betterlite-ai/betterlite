use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::api::responses_poll_state::{PollingSelector, PollingSettings};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{ResponsesApiResponse, decode_responses_objects};
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use indexmap::IndexMap;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use uuid::Uuid;

use crate::api::api_boundaries::ApiLimits;

use super::super::access::{PollAccess, PollDenial};
use super::super::access::{PollCaller, PollOwner};
use super::super::cache::{PollingCache, PollingCacheError};
use super::super::error::PollingError;
use super::super::record::{PollMetadata, PollPatch};
use super::super::selection::{
    DeploymentProjection, DeploymentProvider, PollingRequest, should_use_polling_for_request,
};
use super::super::store::{NewPollRequest, ResponsePollingStore};
use super::super::store::{StoredPollState, decode_responses_poll_state};
use super::super::{
    CACHE_KEY_PREFIX, DEFAULT_TTL, POLLING_ID_PREFIX, generate_polling_id, get_cache_key,
    is_polling_id,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CacheOperation {
    Read,
    Write,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CacheEffect {
    pub(super) operation: CacheOperation,
    pub(super) key: String,
    pub(super) ttl: Option<i64>,
    pub(super) hit: bool,
    pub(super) value: Option<String>,
}

#[derive(Default)]
pub(super) struct FakeCache {
    entries: Mutex<BTreeMap<String, Vec<u8>>>,
    effects: Mutex<Vec<CacheEffect>>,
    failure: Mutex<Option<CacheOperation>>,
}

impl FakeCache {
    pub(super) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(super) fn seed(&self, key: &str, value: &str) {
        self.seed_bytes(key, value.as_bytes());
    }

    pub(super) fn seed_bytes(&self, key: &str, value: &[u8]) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.to_vec());
    }

    pub(super) fn fail_next(&self, operation: CacheOperation) {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(operation);
    }

    pub(super) fn effects(&self) -> Vec<CacheEffect> {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn count(&self, operation: CacheOperation) -> usize {
        self.effects()
            .iter()
            .filter(|effect| effect.operation == operation)
            .count()
    }

    pub(super) fn entry(&self, key: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .map(|bytes| String::from_utf8(bytes.clone()).expect("the stored value is UTF-8"))
    }

    fn failing(&self, operation: CacheOperation) -> bool {
        let mut failure = self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        let matched = *failure == Some(operation);
        if matched {
            *failure = None;
        }
        drop(failure);
        matched
    }

    fn record(&self, effect: CacheEffect) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }

    fn put(&self, key: &str, value: &str) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.as_bytes().to_vec());
    }

    fn take(&self, key: &str) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key)
            .is_some()
    }

    fn check(&self, operation: CacheOperation) -> Result<(), PollingCacheError> {
        if !self.failing(operation) {
            return Ok(());
        }
        let message = "scripted cache failure".to_owned();
        match operation {
            CacheOperation::Read => Err(PollingCacheError::Read(message)),
            CacheOperation::Write => Err(PollingCacheError::Write(message)),
            CacheOperation::Delete => Err(PollingCacheError::Delete(message)),
        }
    }
}

#[async_trait]
impl PollingCache for FakeCache {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError> {
        let value = self.entry(key).map(String::into_bytes);
        self.record(CacheEffect {
            operation: CacheOperation::Read,
            key: key.to_owned(),
            ttl: None,
            hit: value.is_some(),
            value: None,
        });
        self.check(CacheOperation::Read)?;
        Ok(value)
    }

    async fn write(&self, key: &str, value: &str, ttl: i64) -> Result<(), PollingCacheError> {
        let hit = self.entry(key).is_some();
        self.record(CacheEffect {
            operation: CacheOperation::Write,
            key: key.to_owned(),
            ttl: Some(ttl),
            hit,
            value: Some(value.to_owned()),
        });
        self.check(CacheOperation::Write)?;
        self.put(key, value);
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError> {
        self.check(CacheOperation::Delete)?;
        let present = self.take(key);
        self.record(CacheEffect {
            operation: CacheOperation::Delete,
            key: key.to_owned(),
            ttl: None,
            hit: present,
            value: None,
        });
        Ok(i64::from(present))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LogLine {
    pub(super) level: &'static str,
    pub(super) message: String,
}

#[derive(Clone, Default)]
pub(super) struct Watch {
    lines: Arc<Mutex<Vec<LogLine>>>,
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Watch {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARNING",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };
        let mut message = Message::default();
        event.record(&mut message);
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LogLine {
                level,
                message: message.0,
            });
    }
}

impl Watch {
    pub(super) fn messages_at(&self, level: &str) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|line| line.level == level)
            .map(|line| line.message.clone())
            .collect()
    }
}

pub(super) fn watch() -> (DefaultGuard, Watch) {
    let watch = Watch::default();
    let subscriber = registry().with(watch.clone());
    (set_default(subscriber), watch)
}

pub(super) fn store(cache: Arc<FakeCache>, ttl: i64) -> ResponsePollingStore {
    let handle: Option<Arc<dyn PollingCache>> = Some(cache);
    ResponsePollingStore {
        cache: handle,
        ttl,
        limits: ApiLimits::default(),
    }
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture is JSON")
}

pub(super) fn field(value: &Json, key: &str) -> Json {
    let Json::Object(fields) = value else {
        panic!("the value is not a JSON object");
    };
    fields.get(key).cloned().unwrap_or(Json::Null)
}

pub(super) fn has_key(value: &Json, key: &str) -> bool {
    let Json::Object(fields) = value else {
        panic!("the value is not a JSON object");
    };
    fields.contains_key(key)
}

pub(super) fn stored(cache: &FakeCache, polling_id: &str) -> Json {
    json(
        &cache
            .entry(&get_cache_key(polling_id))
            .expect("the key is stored"),
    )
}

pub(super) fn patch(source: &str) -> PollPatch {
    serde_json::from_str(source).expect("the patch is typed")
}

pub(super) fn metadata(pair: (&str, &str)) -> PollMetadata {
    PollMetadata::Fields(IndexMap::from([(
        pair.0.to_owned(),
        Json::Text(pair.1.to_owned()),
    )]))
}

pub(super) fn wire_limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    }
}

pub(super) fn check_b1138(written: &str) {
    let decoded: ResponsesApiResponse = decode_responses_objects(written.as_bytes(), wire_limits())
        .expect("B1138 reads the record");
    assert_eq!(decoded.object.as_deref(), Some("response"));
    assert_eq!(decoded.status.as_deref(), Some("queued"));
    assert_eq!(decoded.created_at, 1_700_000_000);
    assert!(decoded.output.is_empty());
    assert!(decoded.usage.is_none());
    assert!(decoded.model.is_none());
    assert_eq!(
        decoded.metadata,
        Some(BTreeMap::from([(
            "trace".to_owned(),
            Json::Text("a".to_owned())
        )]))
    );
}

pub(super) fn principal(user: Option<&str>, team: Option<&str>, roles: &[&str]) -> Principal {
    let user_json = user.map_or_else(|| "null".to_owned(), |id| format!(r#"{{"id":"{id}"}}"#));
    let teams = team.map_or_else(|| "[]".to_owned(), |id| format!(r#"[{{"id":"{id}"}}]"#));
    let roles_json = roles
        .iter()
        .map(|role| format!(r#""{role}""#))
        .collect::<Vec<String>>()
        .join(",");
    let source = format!(
        r#"{{"principal_type":"human","subject":"subject","auth_method":"api_key","user":{user_json},"teams":{teams},"roles":[{roles_json}]}}"#
    );
    serde_json::from_str(&source).expect("the principal is typed")
}

pub(super) fn caller(
    user: Option<&str>,
    team: Option<&str>,
    admin: bool,
    fingerprint: Option<&str>,
) -> PollCaller {
    PollCaller {
        identity: PollOwner {
            user_id: user.unwrap_or_default().to_owned(),
            team_id: team.unwrap_or_default().to_owned(),
            key_fingerprint: fingerprint.unwrap_or_default().to_owned(),
        },
        platform_admin: admin,
    }
}

pub(super) fn owner(user: &str, team: &str, fingerprint: &str) -> PollOwner {
    PollOwner {
        user_id: user.to_owned(),
        team_id: team.to_owned(),
        key_fingerprint: fingerprint.to_owned(),
    }
}

pub(super) fn assert_denial(error: PollingError, denial: PollDenial) {
    match error {
        PollingError::Forbidden(found) => assert_eq!(found, denial),
        other @ (PollingError::Cache(_) | PollingError::Decode(_) | PollingError::Encode(_)) => {
            panic!("expected a forbidden polling error, got {other}")
        }
    }
}

pub(super) fn keys_and_ids() {
    let uuid = Uuid::parse_str("00000000-0000-4000-8000-000000000001").expect("the UUID parses");
    let polling_id = generate_polling_id(uuid);
    assert_eq!(
        polling_id,
        "litellm_poll_00000000-0000-4000-8000-000000000001"
    );
    assert_eq!(POLLING_ID_PREFIX, "litellm_poll_");
    assert_eq!(CACHE_KEY_PREFIX, "litellm:polling:response:");
    assert_eq!(
        get_cache_key(&polling_id),
        "litellm:polling:response:litellm_poll_00000000-0000-4000-8000-000000000001"
    );
    assert_eq!(DEFAULT_TTL, 3600_i64);
    assert!(is_polling_id("litellm_poll_"));
    assert!(is_polling_id("litellm_poll_not-a-uuid"));
    assert!(!is_polling_id("resp_a"));
}

pub(super) fn settings(selector: PollingSelector, native: &[&str]) -> PollingSettings {
    PollingSettings {
        polling_via_cache: selector,
        native_background_mode: native.iter().map(|model| (*model).to_owned()).collect(),
        ttl: DEFAULT_TTL,
    }
}

pub(super) fn providers(names: &[&str]) -> PollingSettings {
    settings(
        PollingSelector::Providers(names.iter().map(|name| (*name).to_owned()).collect()),
        &[],
    )
}

pub(super) fn deployment_list<'entry>(
    entries: &'entry [(Option<&'entry str>, Option<&'entry str>)],
) -> Vec<DeploymentProvider<'entry>> {
    entries
        .iter()
        .map(|(custom, model)| DeploymentProvider {
            custom_llm_provider: *custom,
            model: *model,
        })
        .collect()
}

pub(super) fn projection<'router>(
    indices: &'router [usize],
    deployments: &'router [DeploymentProvider<'router>],
) -> DeploymentProjection<'router> {
    DeploymentProjection {
        indices,
        deployments,
    }
}

pub(super) fn stored_state(cache: &FakeCache, polling_id: &str) -> StoredPollState {
    let bytes = cache
        .entry(&get_cache_key(polling_id))
        .expect("the key is stored");
    decode_responses_poll_state(bytes.as_bytes(), ApiLimits::default()).expect("the state decodes")
}

pub(super) fn new_request(polling_id: &str, owner: Option<PollOwner>) -> NewPollRequest<'_> {
    NewPollRequest {
        polling_id,
        created_at: 1_700_000_000,
        metadata: None,
        owner,
    }
}

pub(super) fn access(caller: &PollCaller) -> PollAccess<'_> {
    PollAccess::Caller(caller)
}

pub(super) fn selection_enabled(
    background: bool,
    model: &str,
    settings: &PollingSettings,
    router: Option<&DeploymentProjection<'_>>,
) -> bool {
    let request = PollingRequest {
        background,
        model,
        redis_available: true,
    };
    should_use_polling_for_request(request, settings, router)
}
