use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_auth::resolved_key::auth::{AuthInput, ParentOtelSpan, UserAPIKeyAuth};
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::structured::Payload;
use chrono::{DateTime, FixedOffset};
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::legacy_admission::error::LegacyAdmissionError;
use crate::legacy_admission::{AdmissionRequest, LegacyAdmission};
use crate::rate_limiters::legacy_rate_limit_counters::LegacyRateLimitHandler;
use crate::rate_limiters::legacy_rate_limit_counters::clock::LegacyClock;
use crate::rate_limiters::legacy_rate_limit_counters::error::LegacyRateLimitError;
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::{
    LegacyCacheValue, LegacyUsageCache, MinuteCounter, UsageCacheError,
};
use crate::rate_limiters::legacy_rate_limit_counters::verbose::VerboseConsole;

pub(crate) const START_OF_MINUTE: &str = "2026-11-21T12:00:00+09:00";
pub(crate) const MINUTE: &str = "2026-11-21-12-00";
pub(crate) const FULL_KEY: &str =
    r#"{"api_key":"key-1","user_id":"user-1","team_id":"team-1","end_user_id":"end-user-1"}"#;
pub(crate) const NEAR_MAX: i64 = i64::MAX.saturating_sub(1);

// every cache call the hook makes, in order
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CacheCall {
    BatchGet(Vec<Option<String>>),
    BatchSet {
        entries: Vec<(String, LegacyCacheValue)>,
        ttl: u64,
        local_only: bool,
    },
}

// what a batch read answers: a failed read, no values, or one value per key
#[derive(Clone, Debug)]
pub(crate) enum BatchAnswer {
    None,
    Values(Vec<Option<LegacyCacheValue>>),
    Failed,
}

// which writes a boxed cache refuses, so one failing write leaves the earlier ones intact
#[derive(Clone, Copy)]
pub(crate) enum WriteFailure {
    Never,
    LocalOnly,
    RemoteOnly,
}

pub(crate) struct RecordingCache {
    calls: Mutex<Vec<CacheCall>>,
    global_answer: Mutex<BatchAnswer>,
    objects_answer: Mutex<BatchAnswer>,
    write_failure: WriteFailure,
}

impl RecordingCache {
    pub(crate) fn new() -> Arc<Self> {
        Self::with_write_failure(WriteFailure::Never)
    }

    pub(crate) fn with_reads(global_answer: BatchAnswer, objects_answer: BatchAnswer) -> Arc<Self> {
        let cache = Self::new();
        *cache.global_answer.lock().expect("the cache lock") = global_answer;
        *cache.objects_answer.lock().expect("the cache lock") = objects_answer;
        cache
    }

    pub(crate) fn with_write_failure(failure: WriteFailure) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            global_answer: Mutex::new(BatchAnswer::None),
            objects_answer: Mutex::new(BatchAnswer::None),
            write_failure: failure,
        })
    }

    pub(crate) fn calls(&self) -> Vec<CacheCall> {
        self.calls.lock().expect("the cache lock").clone()
    }

    fn answer(&self, key_count: usize) -> &Mutex<BatchAnswer> {
        if key_count == 1 {
            return &self.global_answer;
        }
        &self.objects_answer
    }

    fn refuse(&self, local_only: bool) -> bool {
        match self.write_failure {
            WriteFailure::Never => false,
            WriteFailure::LocalOnly => local_only,
            WriteFailure::RemoteOnly => !local_only,
        }
    }

    fn record(&self, call: CacheCall) {
        self.calls.lock().expect("the cache lock").push(call);
    }
}

#[async_trait]
impl LegacyUsageCache for RecordingCache {
    async fn batch_get(
        &self,
        keys: &[Option<String>],
        _parent_otel_span: Option<&ParentOtelSpan>,
    ) -> Result<Option<Vec<Option<LegacyCacheValue>>>, UsageCacheError> {
        self.record(CacheCall::BatchGet(keys.to_vec()));
        let answer = self
            .answer(keys.len())
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match answer {
            BatchAnswer::None => Ok(None),
            BatchAnswer::Values(values) => Ok(Some(values)),
            BatchAnswer::Failed => Err(UsageCacheError::Transport(
                DualCacheError::UnrepresentableValue,
            )),
        }
    }

    async fn batch_set(
        &self,
        entries: &[(String, LegacyCacheValue)],
        ttl: u64,
        _parent_otel_span: Option<&ParentOtelSpan>,
        local_only: bool,
    ) -> Result<(), UsageCacheError> {
        self.record(CacheCall::BatchSet {
            entries: entries.to_vec(),
            ttl,
            local_only,
        });
        if self.refuse(local_only) {
            return Err(UsageCacheError::Transport(
                DualCacheError::UnrepresentableValue,
            ));
        }
        Ok(())
    }
}

// the process clock, pinned to one instant, counting the reads the hook makes
pub(crate) struct FixedClock {
    now: DateTime<FixedOffset>,
    reads: AtomicUsize,
}

impl FixedClock {
    pub(crate) fn at(text: &str) -> Arc<Self> {
        Arc::new(Self {
            now: DateTime::parse_from_rfc3339(text).expect("the fixture is a timestamp"),
            reads: AtomicUsize::new(0),
        })
    }

    pub(crate) fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }
}

impl LegacyClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.now
    }
}

pub(crate) struct RecordingConsole {
    printed: Mutex<Vec<String>>,
}

impl RecordingConsole {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            printed: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn printed(&self) -> Vec<String> {
        self.printed.lock().expect("the console lock").clone()
    }
}

impl VerboseConsole for RecordingConsole {
    fn print_line(&self, statement: &str) -> bool {
        self.printed
            .lock()
            .expect("the console lock")
            .push(statement.to_owned());
        true
    }
}

// the native provider registry: no model resolves, so a 429 keeps its unresolved model and provider
struct NoProviders;

impl ProviderResolution for NoProviders {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}

pub(crate) fn handler_with(
    cache: &Arc<RecordingCache>,
    clock: &Arc<FixedClock>,
    console: &Arc<RecordingConsole>,
) -> LegacyRateLimitHandler {
    let usage_cache_arc: Arc<dyn LegacyUsageCache> = Arc::<RecordingCache>::clone(cache);
    let wall_clock_arc: Arc<dyn LegacyClock> = Arc::<FixedClock>::clone(clock);
    let verbose_console_arc: Arc<dyn VerboseConsole> = Arc::<RecordingConsole>::clone(console);
    LegacyRateLimitHandler {
        internal_usage_cache: usage_cache_arc,
        clock: wall_clock_arc,
        verbose: verbose_console_arc,
        set_verbose: true,
        native: Arc::new(NoProviders),
        router: None,
    }
}

pub(crate) struct Fixture {
    pub(crate) cache: Arc<RecordingCache>,
    pub(crate) clock: Arc<FixedClock>,
    pub(crate) console: Arc<RecordingConsole>,
    pub(crate) admission: LegacyAdmission,
}

pub(crate) fn fixture_with(cache: Arc<RecordingCache>, clock: Arc<FixedClock>) -> Fixture {
    let console = RecordingConsole::new();
    let handler = handler_with(&cache, &clock, &console);
    Fixture {
        cache,
        clock,
        console,
        admission: LegacyAdmission { handler },
    }
}

pub(crate) fn new_fixture() -> Fixture {
    fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE))
}

// a fixture whose global read and six-key read answer as given
pub(crate) fn answered(global_answer: BatchAnswer, objects_answer: BatchAnswer) -> Fixture {
    fixture_with(
        RecordingCache::with_reads(global_answer, objects_answer),
        FixedClock::at(START_OF_MINUTE),
    )
}

pub(crate) fn objects(values: &[Option<LegacyCacheValue>]) -> BatchAnswer {
    BatchAnswer::Values(values.to_vec())
}

// the six-key read answering one counter for the key, the key-model pair, the user, the team and the end user
pub(crate) fn every_dimension(current: LegacyCacheValue) -> BatchAnswer {
    objects(&[
        None,
        Some(current),
        Some(current),
        Some(current),
        Some(current),
        Some(current),
    ])
}

pub(crate) fn global_count(count: i64) -> BatchAnswer {
    BatchAnswer::Values(vec![Some(LegacyCacheValue::Count(count))])
}

pub(crate) fn minute(requests: i64, tpm: i64, rpm: i64) -> LegacyCacheValue {
    LegacyCacheValue::Counter(MinuteCounter {
        current_requests: requests,
        current_tpm: tpm,
        current_rpm: rpm,
    })
}

pub(crate) fn principal(text: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(text).expect("the fixture is an object");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the fixture is a valid auth record")
}

pub(crate) fn hashed_key(auth: &UserAPIKeyAuth) -> String {
    account_limits(auth)
        .hashed_api_key
        .expect("the fixture carries an api key")
        .to_owned()
}

pub(crate) fn request_body(model: Option<&str>, metadata: &str) -> AdmissionRequest {
    AdmissionRequest {
        model: model.map(str::to_owned),
        metadata: serde_json::from_str::<JsonMap>(metadata).expect("the fixture is an object"),
    }
}

pub(crate) fn rejection(result: Result<(), LegacyAdmissionError>) -> ProxyRateLimitError {
    match result {
        Err(LegacyAdmissionError::RateLimit(LegacyRateLimitError::Rejected(error))) => *error,
        other => panic!("expected a 429, got {other:?}"),
    }
}

pub(crate) fn detail_text(error: &ProxyRateLimitError) -> String {
    match &error.detail {
        Payload::String(text) => text.clone(),
        other @ Payload::Null
        | other @ Payload::Bool(_)
        | other @ Payload::Number(_)
        | other @ Payload::Array(_)
        | other @ Payload::Object(_) => panic!("expected a string detail, got {other:?}"),
    }
}

// one batch write: its entries, ttl and local-only flag
pub(crate) type BatchSetCall = (Vec<(String, LegacyCacheValue)>, u64, bool);

// the hook's calls split by kind, each list in call order
#[derive(Default)]
pub(crate) struct SplitCalls {
    pub(crate) gets: Vec<Vec<Option<String>>>,
    pub(crate) sets: Vec<BatchSetCall>,
}

pub(crate) fn split_calls(calls: &[CacheCall]) -> SplitCalls {
    let mut split = SplitCalls::default();
    for call in calls {
        match call {
            CacheCall::BatchGet(keys) => split.gets.push(keys.clone()),
            CacheCall::BatchSet {
                entries,
                ttl,
                local_only,
            } => split.sets.push((entries.clone(), *ttl, *local_only)),
        }
    }
    split
}

pub(crate) fn final_write(calls: &[CacheCall]) -> BatchSetCall {
    let mut sets = split_calls(calls).sets;
    sets.pop().expect("the hook writes the final batch")
}

pub(crate) struct Caller {
    pub(crate) auth: UserAPIKeyAuth,
    pub(crate) hash: String,
}

pub(crate) fn auth_with_hash(text: &str) -> Caller {
    let auth = principal(text);
    let hash = hashed_key(&auth);
    Caller { auth, hash }
}

pub(crate) fn id(part: &str) -> String {
    format!("{part}::{MINUTE}::request_count")
}

// every counter id the hook wrote, in call order
pub(crate) fn written_ids(calls: &[CacheCall]) -> Vec<String> {
    let mut ids = Vec::new();
    for (entries, _, _) in split_calls(calls).sets {
        for (counter_id, _) in entries {
            ids.push(counter_id);
        }
    }
    ids
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct LogCapture {
    buffer: Arc<Mutex<Vec<u8>>>,
    _guard: DefaultGuard,
}

impl LogCapture {
    pub(crate) fn start() -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buffer);
        let subscriber = fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(Level::TRACE)
            .with_writer(move || Captured(Arc::clone(&sink)))
            .finish();
        Self {
            buffer,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn text(&self) -> String {
        let bytes = self.buffer.lock().expect("the capture lock").clone();
        String::from_utf8(bytes).expect("the capture is UTF-8")
    }
}
