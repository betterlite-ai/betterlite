use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::combined_key_read::error::KeyReadError;
use betterlite_db::combined_key_read::view::CombinedKeyView;
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::auth_invalidation::error::InvalidationError;
use crate::auth_key_lookup::cache::KeyCacheWriter;
use crate::auth_key_lookup::permission::KeyPermissionLookup;
use crate::auth_key_lookup::reader::{KeyReader, KeySource};
use crate::auth_prefetch::permission::lookup::PermissionError;
use crate::identity_store::IdentityStore;
use crate::identity_store::ports::{Dependencies, KeyObjectCache};
use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};

pub(super) const KEY_ROW: &str =
    r#"{"token":"hashed-token","key_name":"key-1","user_id":"u1","models":["gpt-4"],"spend":1.5}"#;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn auth_from_fields(text: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(text).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn view_from_text(text: &str) -> CombinedKeyView {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(text).unwrap();
    CombinedKeyView::construct(fields, Decimal::ZERO).unwrap()
}

pub(super) fn permission_table() -> LiteLLMObjectPermissionTable {
    serde_json::from_str(r#"{"object_permission_id":"op-1","models":["gpt-4"]}"#).unwrap()
}

pub(super) fn span(marker: u8) -> SpanContext {
    SpanContext::new(
        TraceId::from_bytes([marker; 16]),
        SpanId::from_bytes([marker; 8]),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

pub(super) struct FakeSource {
    answers: Mutex<VecDeque<Result<Option<CombinedKeyView>, KeyReadError>>>,
    calls: Mutex<Vec<(String, bool, Option<SpanContext>)>>,
}

impl FakeSource {
    pub(super) fn new(answers: Vec<Result<Option<CombinedKeyView>, KeyReadError>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(VecDeque::from(answers)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<(String, bool, Option<SpanContext>)> {
        lock(&self.calls).clone()
    }
}

#[async_trait]
impl KeySource for FakeSource {
    async fn read_key(
        &self,
        hashed_token: &str,
        use_writer: bool,
        parent_otel_span: Option<SpanContext>,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        lock(&self.calls).push((hashed_token.to_owned(), use_writer, parent_otel_span));
        lock(&self.answers).pop_front().unwrap_or(Ok(None))
    }
}

pub(super) fn reader(source: Arc<FakeSource>) -> Arc<KeyReader> {
    Arc::new(KeyReader {
        source,
        reconnect: None,
        gate: Arc::new(LoopBoundSemaphore {
            value: 8,
            slot: Mutex::new(None),
        }),
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
        default_deadline_seconds: 5.0,
    })
}

pub(super) enum CacheAnswer {
    Miss,
    Hit(Box<UserAPIKeyAuth>),
    Error,
}

pub(super) struct FakeCache {
    answer: Mutex<CacheAnswer>,
    reads: Mutex<Vec<String>>,
}

impl FakeCache {
    pub(super) fn new(answer: CacheAnswer) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(answer),
            reads: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn reads(&self) -> Vec<String> {
        lock(&self.reads).clone()
    }

    pub(super) fn hit(&self) -> Option<UserAPIKeyAuth> {
        match &*lock(&self.answer) {
            CacheAnswer::Miss | CacheAnswer::Error => None,
            CacheAnswer::Hit(key) => Some((**key).clone()),
        }
    }
}

#[async_trait]
impl KeyObjectCache for FakeCache {
    async fn read_key_object(
        &self,
        hashed_token: &str,
    ) -> Result<Option<UserAPIKeyAuth>, AuthError> {
        lock(&self.reads).push(hashed_token.to_owned());
        match &*lock(&self.answer) {
            CacheAnswer::Miss => Ok(None),
            CacheAnswer::Hit(key) => Ok(Some((**key).clone())),
            CacheAnswer::Error => Err(AuthError::NonStringApiKey),
        }
    }
}

pub(super) struct FakePermissions {
    answers: Mutex<VecDeque<Result<Option<LiteLLMObjectPermissionTable>, PermissionError>>>,
    calls: Mutex<Vec<(String, bool)>>,
}

impl FakePermissions {
    pub(super) fn new(
        answers: Vec<Result<Option<LiteLLMObjectPermissionTable>, PermissionError>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(VecDeque::from(answers)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<(String, bool)> {
        lock(&self.calls).clone()
    }
}

#[async_trait]
impl KeyPermissionLookup for FakePermissions {
    async fn permission(
        &self,
        object_permission_id: &str,
        check_db_only: bool,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        lock(&self.calls).push((object_permission_id.to_owned(), check_db_only));
        lock(&self.answers).pop_front().unwrap_or(Ok(None))
    }
}

pub(super) struct FakeWriter {
    fail: bool,
    writes: Mutex<Vec<String>>,
    seen_permission: Mutex<Vec<bool>>,
}

impl FakeWriter {
    pub(super) fn new(fail: bool) -> Arc<Self> {
        Arc::new(Self {
            fail,
            writes: Mutex::new(Vec::new()),
            seen_permission: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn writes(&self) -> Vec<String> {
        lock(&self.writes).clone()
    }

    pub(super) fn seen_permission(&self) -> Vec<bool> {
        lock(&self.seen_permission).clone()
    }
}

#[async_trait]
impl KeyCacheWriter for FakeWriter {
    async fn write_key(
        &self,
        hashed_token: &str,
        auth: &mut UserAPIKeyAuth,
    ) -> Result<(), InvalidationError> {
        lock(&self.writes).push(hashed_token.to_owned());
        lock(&self.seen_permission).push(auth.view.token.object_permission.is_some());
        auth.view.last_refreshed_at = Some(Decimal::ONE);
        if self.fail {
            return Err(InvalidationError::Timestamp(Decimal::ZERO));
        }
        Ok(())
    }
}

pub(super) struct StoreSetup {
    pub(super) reader: Option<Arc<KeyReader>>,
    pub(super) cache: Arc<FakeCache>,
    pub(super) permissions: Arc<FakePermissions>,
    pub(super) writer: Arc<FakeWriter>,
    pub(super) span: Option<SpanContext>,
    pub(super) check_cache_only: bool,
}

impl StoreSetup {
    pub(super) fn store(&self) -> IdentityStore {
        IdentityStore::new(
            Dependencies {
                reader: self.reader.clone(),
                cache: Arc::clone(&self.cache),
                permissions: Arc::clone(&self.permissions),
                writer: Arc::clone(&self.writer),
            },
            self.span.clone(),
            self.check_cache_only,
        )
    }
}

pub(super) fn setup() -> StoreSetup {
    StoreSetup {
        reader: None,
        cache: FakeCache::new(CacheAnswer::Miss),
        permissions: FakePermissions::new(Vec::new()),
        writer: FakeWriter::new(false),
        span: None,
        check_cache_only: false,
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(super) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(logs.clone())
            .finish();
        (logs, set_default(subscriber))
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(lock(&self.0).clone()).expect("the captured logs are UTF-8")
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}
