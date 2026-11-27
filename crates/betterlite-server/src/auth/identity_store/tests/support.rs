use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_auth::auth_key_lookup::cache::KeyCacheWriter;
use betterlite_auth::auth_key_lookup::permission::KeyPermissionLookup;
use betterlite_auth::auth_key_lookup::reader::{KeyReader, KeySource};
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::identity_store::ports::{KeyObjectCache, Ports};
use betterlite_auth::resolved_key::auth::{AuthError, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::combined_key_read::error::KeyReadError;
use betterlite_db::combined_key_read::view::CombinedKeyView;
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use opentelemetry::trace::SpanContext;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

pub(super) const KEY_ROW: &str =
    r#"{"token":"hashed-token","key_name":"key-1","user_id":"u1","models":["gpt-4"],"spend":1.5}"#;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn view_from_text(text: &str) -> CombinedKeyView {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(text).unwrap();
    CombinedKeyView::construct(fields, Decimal::ZERO).unwrap()
}

struct FakeSource {
    row: Mutex<Option<CombinedKeyView>>,
}

#[async_trait]
impl KeySource for FakeSource {
    async fn read_key(
        &self,
        _hashed_token: &str,
        _use_writer: bool,
        _parent_otel_span: Option<SpanContext>,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        Ok(lock(&self.row).clone())
    }
}

pub(super) fn reader(row: Option<CombinedKeyView>) -> Arc<KeyReader> {
    Arc::new(KeyReader {
        source: Arc::new(FakeSource {
            row: Mutex::new(row),
        }),
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

struct MissCache;

#[async_trait]
impl KeyObjectCache for MissCache {
    async fn read_key_object(
        &self,
        _hashed_token: &str,
    ) -> Result<Option<UserAPIKeyAuth>, AuthError> {
        Ok(None)
    }
}

struct NoPermissions;

#[async_trait]
impl KeyPermissionLookup for NoPermissions {
    async fn permission(
        &self,
        _object_permission_id: &str,
        _check_db_only: bool,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        Ok(None)
    }
}

#[derive(Default)]
pub(super) struct RecordingWriter {
    writes: Mutex<Vec<String>>,
}

impl RecordingWriter {
    pub(super) fn writes(&self) -> Vec<String> {
        lock(&self.writes).clone()
    }
}

#[async_trait]
impl KeyCacheWriter for RecordingWriter {
    async fn write_key(
        &self,
        hashed_token: &str,
        _auth: &mut UserAPIKeyAuth,
    ) -> Result<(), InvalidationError> {
        lock(&self.writes).push(hashed_token.to_owned());
        Ok(())
    }
}

pub(super) fn ports(reader: Option<Arc<KeyReader>>) -> (Ports, Arc<RecordingWriter>) {
    let writer = Arc::new(RecordingWriter::default());
    let cache: Arc<dyn KeyObjectCache> = Arc::new(MissCache);
    let permissions: Arc<dyn KeyPermissionLookup> = Arc::new(NoPermissions);
    let writer_port: Arc<dyn KeyCacheWriter> = Arc::clone(&writer);
    (
        Ports {
            reader,
            cache: Some(cache),
            permissions: Some(permissions),
            writer: Some(writer_port),
        },
        writer,
    )
}
