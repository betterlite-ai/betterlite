use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_auth::auth_key_lookup::KeyLookups;
use betterlite_auth::auth_key_lookup::cache::{KeyCache, KeyCacheWriter};
use betterlite_auth::auth_key_lookup::permission::KeyPermissionLookup;
use betterlite_auth::auth_key_lookup::reader::{KeyReader, KeySource};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::combined_key_read::error::{DatabaseFailure, KeyReadError};
use betterlite_db::combined_key_read::view::CombinedKeyView;
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_db::database_faults::reconnect::DbReconnect;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use opentelemetry::trace::SpanContext;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::task::TaskTracker;

use super::permissions::{ScriptedPermissions, permission_fixture};
use super::{KeyObjectInput, RowStep};

type ReconnectCalls = Arc<Mutex<Vec<(String, f64, f64)>>>;

fn transport_failure() -> KeyReadError {
    KeyReadError::Database(DatabaseFailure::Reported(DbError {
        error_type: ErrorType::Connect,
        message: "connection closed".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }))
}

pub(super) struct ScriptedSource {
    answers: Mutex<VecDeque<Result<Option<CombinedKeyView>, KeyReadError>>>,
    calls: Mutex<Vec<bool>>,
}

impl ScriptedSource {
    pub(super) fn calls(&self) -> Vec<bool> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl KeySource for ScriptedSource {
    async fn read_key(
        &self,
        _hashed_token: &str,
        use_writer: bool,
        _parent_otel_span: Option<SpanContext>,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(use_writer);
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(None))
    }
}

struct ScriptedReconnect {
    timeout: Option<f64>,
    lock: Option<f64>,
    answer: bool,
    calls: ReconnectCalls,
}

impl DbReconnect for ScriptedReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        self.timeout
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        self.lock
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((reason.to_owned(), timeout_seconds, lock_timeout_seconds));
            Ok(self.answer)
        })
    }
}

struct ScriptedCache {
    hit: Option<UserAPIKeyAuth>,
}

#[async_trait]
impl KeyCache for ScriptedCache {
    async fn read_key(&self, _hashed_token: &str) -> Option<UserAPIKeyAuth> {
        self.hit.clone()
    }
}

pub(super) struct ScriptedWriter {
    pub(super) writes: Mutex<usize>,
}

#[async_trait]
impl KeyCacheWriter for ScriptedWriter {
    async fn write_key(
        &self,
        _hashed_token: &str,
        _auth: &mut UserAPIKeyAuth,
    ) -> Result<(), InvalidationError> {
        let mut writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        *writes = writes.saturating_add(1);
        Ok(())
    }
}

pub(super) struct ReadFixture {
    pub(super) source: Arc<ScriptedSource>,
    pub(super) writer: Arc<ScriptedWriter>,
    reader: Option<Arc<KeyReader>>,
    cache: Arc<ScriptedCache>,
    pub(super) permissions: Arc<ScriptedPermissions>,
    reconnect_calls: ReconnectCalls,
}

impl ReadFixture {
    pub(super) fn lookups(&self) -> KeyLookups {
        let cache: Arc<dyn KeyCache> = Arc::<ScriptedCache>::clone(&self.cache);
        let writer: Arc<dyn KeyCacheWriter> = Arc::<ScriptedWriter>::clone(&self.writer);
        let permissions: Arc<dyn KeyPermissionLookup> =
            Arc::<ScriptedPermissions>::clone(&self.permissions);
        KeyLookups {
            reader: self.reader.as_ref().map(Arc::clone),
            cache,
            writer,
            permissions: Some(permissions),
        }
    }

    pub(super) fn reconnect_calls(&self) -> Value {
        let calls = self
            .reconnect_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Value::Array(
            calls
                .into_iter()
                .map(|(reason, timeout, lock)| {
                    json!({"reason": reason, "timeout": timeout, "lock": lock})
                })
                .collect(),
        )
    }
}

pub(super) fn read_fixture(input: &KeyObjectInput) -> Result<ReadFixture, Box<HelperError>> {
    let source = Arc::new(ScriptedSource {
        answers: Mutex::new(read_answers(&input.rows)?),
        calls: Mutex::new(Vec::new()),
    });
    let reconnect_calls: ReconnectCalls = Arc::new(Mutex::new(Vec::new()));
    let reconnect_client: Box<dyn DbReconnect + Send> = Box::new(ScriptedReconnect {
        timeout: input.reconnect.timeout,
        lock: input.reconnect.lock,
        answer: input.reconnect.answer,
        calls: Arc::clone(&reconnect_calls),
    });
    let reconnect = Some(Arc::new(AsyncMutex::new(reconnect_client)));
    Ok(ReadFixture {
        reader: Some(reader(Arc::clone(&source), reconnect)),
        source,
        cache: Arc::new(ScriptedCache {
            hit: cached_auth(input.cached.as_ref())?,
        }),
        writer: Arc::new(ScriptedWriter {
            writes: Mutex::new(0),
        }),
        permissions: permission_fixture(input.permissions.as_ref())?,
        reconnect_calls,
    })
}

fn read_answers(
    steps: &[RowStep],
) -> Result<VecDeque<Result<Option<CombinedKeyView>, KeyReadError>>, Box<HelperError>> {
    let mut answers = VecDeque::new();
    for step in steps {
        let answer = match step {
            RowStep::Row { fields } => {
                let row_fields: BTreeMap<String, JsonData> =
                    serde_json::from_value(fields.clone()).or_value_error()?;
                Ok(Some(
                    CombinedKeyView::construct(row_fields, Decimal::ZERO).or_value_error()?,
                ))
            }
            RowStep::Transport => Err(transport_failure()),
        };
        answers.push_back(answer);
    }
    Ok(answers)
}

fn cached_auth(stored: Option<&Value>) -> Result<Option<UserAPIKeyAuth>, Box<HelperError>> {
    let Some(cached) = stored else {
        return Ok(None);
    };
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_value(cached.clone()).or_value_error()?;
    Ok(Some(
        UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?,
    ))
}

fn reader(
    source: Arc<ScriptedSource>,
    reconnect: Option<Arc<AsyncMutex<Box<dyn DbReconnect + Send>>>>,
) -> Arc<KeyReader> {
    Arc::new(KeyReader {
        source,
        reconnect,
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
