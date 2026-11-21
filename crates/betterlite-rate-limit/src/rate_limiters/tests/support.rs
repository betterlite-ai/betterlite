use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution, SnapshotError};
use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use chrono::{DateTime, FixedOffset};
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::rate_limiters::legacy_rate_limit_counters::LegacyRateLimitDeps;
use crate::rate_limiters::legacy_rate_limit_counters::clock::LegacyClock;
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::{
    LegacyCacheValue, LegacyUsageCache, UsageCacheError,
};
use crate::rate_limiters::legacy_rate_limit_counters::verbose::VerboseConsole;

// every cache call the handler makes, in order
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CacheCall {
    BatchGet {
        keys: Vec<Option<String>>,
        span: bool,
    },
    BatchSet {
        entries: Vec<(String, LegacyCacheValue)>,
        ttl: u64,
        local_only: bool,
        span: bool,
    },
}

// what the next batch read answers: the handler must treat all three as no counter
#[derive(Clone, Debug)]
pub(crate) enum BatchAnswer {
    None,
    Values(Vec<Option<LegacyCacheValue>>),
    Failed,
}

pub(crate) struct RecordingCache {
    calls: Mutex<Vec<CacheCall>>,
    answer: Mutex<BatchAnswer>,
    write_fails: AtomicBool,
}

impl RecordingCache {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            answer: Mutex::new(BatchAnswer::None),
            write_fails: AtomicBool::new(false),
        })
    }

    pub(crate) fn answering(answer: BatchAnswer) -> Arc<Self> {
        let cache = Self::new();
        *cache.answer.lock().expect("the cache lock") = answer;
        cache
    }

    pub(crate) fn with_failing_writes() -> Arc<Self> {
        let cache = Self::new();
        cache.write_fails.store(true, Ordering::Relaxed);
        cache
    }

    pub(crate) fn calls(&self) -> Vec<CacheCall> {
        self.calls.lock().expect("the cache lock").clone()
    }

    fn record(&self, call: CacheCall) {
        self.calls.lock().expect("the cache lock").push(call);
    }
}

fn cache_failure() -> UsageCacheError {
    UsageCacheError::Failed(Box::new(io::Error::other("the cache is down")))
}

#[async_trait]
impl LegacyUsageCache for RecordingCache {
    async fn batch_get(
        &self,
        keys: &[Option<String>],
        parent_otel_span: Option<&ParentOtelSpan>,
    ) -> Result<Option<Vec<Option<LegacyCacheValue>>>, UsageCacheError> {
        self.record(CacheCall::BatchGet {
            keys: keys.to_vec(),
            span: parent_otel_span.is_some(),
        });
        let answer = self
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match answer {
            BatchAnswer::None => Ok(None),
            BatchAnswer::Values(values) => Ok(Some(values)),
            BatchAnswer::Failed => Err(cache_failure()),
        }
    }

    async fn batch_set(
        &self,
        entries: &[(String, LegacyCacheValue)],
        ttl: u64,
        parent_otel_span: Option<&ParentOtelSpan>,
        local_only: bool,
    ) -> Result<(), UsageCacheError> {
        self.record(CacheCall::BatchSet {
            entries: entries.to_vec(),
            ttl,
            local_only,
            span: parent_otel_span.is_some(),
        });
        if self.write_fails.load(Ordering::Relaxed) {
            return Err(cache_failure());
        }
        Ok(())
    }
}

// the local wall clock, pinned to one instant, counting the reads the handler makes
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
    fails: bool,
}

impl RecordingConsole {
    pub(crate) fn new(fails: bool) -> Arc<Self> {
        Arc::new(Self {
            printed: Mutex::new(Vec::new()),
            fails,
        })
    }

    pub(crate) fn printed(&self) -> Vec<String> {
        self.printed.lock().expect("the console lock").clone()
    }
}

impl VerboseConsole for RecordingConsole {
    fn print_line(&self, statement: &str) -> bool {
        if self.fails {
            return false;
        }
        self.printed
            .lock()
            .expect("the console lock")
            .push(statement.to_owned());
        true
    }
}

// a console whose writes always fail, for the system console's own error path
pub(crate) struct BrokenConsole;

impl io::Write for &BrokenConsole {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("the console is gone"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// the native provider registry: one model resolves, everything else fails
pub(crate) struct Native;

impl ProviderResolution for Native {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        if model != "openai/gpt-4o-mini" {
            return Err(ResolutionError::MissingProvider {
                model: model.to_owned(),
            });
        }
        Ok(ResolvedModel {
            model: String::from("gpt-4o-mini"),
            provider: ProviderIdentity::Custom(String::from("openai")),
            api_base: None,
            api_key: None,
        })
    }
}

pub(crate) struct FixedSnapshot {
    rows: Vec<Json>,
}

impl FixedSnapshot {
    pub(crate) fn new(rows: Vec<Json>) -> Arc<Self> {
        Arc::new(Self { rows })
    }
}

impl DeploymentSnapshot for FixedSnapshot {
    fn model_list(&self) -> Result<Option<&[Json]>, SnapshotError> {
        Ok(Some(self.rows.as_slice()))
    }
}

pub(crate) fn handler_deps(
    cache: &Arc<RecordingCache>,
    clock: &Arc<FixedClock>,
    console: &Arc<RecordingConsole>,
) -> LegacyRateLimitDeps {
    let usage_cache_arc: Arc<dyn LegacyUsageCache> = Arc::<RecordingCache>::clone(cache);
    let wall_clock_arc: Arc<dyn LegacyClock> = Arc::<FixedClock>::clone(clock);
    let verbose_console_arc: Arc<dyn VerboseConsole> = Arc::<RecordingConsole>::clone(console);
    LegacyRateLimitDeps {
        internal_usage_cache: usage_cache_arc,
        clock: wall_clock_arc,
        verbose: verbose_console_arc,
        set_verbose: false,
        native: Arc::new(Native),
        router: None,
    }
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
