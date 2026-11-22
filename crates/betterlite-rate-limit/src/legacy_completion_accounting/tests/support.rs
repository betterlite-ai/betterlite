use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};
use chrono::{DateTime, FixedOffset};
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::legacy_completion_accounting::counter_cache::CounterCache;
use crate::rate_limiters::legacy_rate_limit_counters::LegacyRateLimitHandler;
use crate::rate_limiters::legacy_rate_limit_counters::clock::LegacyClock;
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::{
    LegacyCacheValue, LegacyUsageCache, MinuteCounter, UsageCacheError,
};
use crate::rate_limiters::legacy_rate_limit_counters::verbose::VerboseConsole;

// the minute every fixture runs in: 2026-11-21 09:05 local time at UTC+01:00
pub(crate) const MINUTE: &str = "2026-11-21-09-05";

// a failing entry "key" fails every call on that key and "method key" only that method; this name fails the batch write
pub(crate) const BATCH_WRITE: &str = "async_batch_set_cache";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Call {
    Get {
        key: String,
        local_only: bool,
    },
    Set {
        key: String,
        value: LegacyCacheValue,
        ttl: u64,
    },
    Increment {
        key: String,
        delta: i64,
        local_only: bool,
    },
    BatchGet {
        keys: Vec<Option<String>>,
    },
    BatchSet {
        entries: Vec<(String, LegacyCacheValue)>,
        ttl: u64,
        local_only: bool,
    },
}

// one store behind both cache traits, recording every call in order
pub(crate) struct RecordingCache {
    stored: Mutex<BTreeMap<String, LegacyCacheValue>>,
    failing: Vec<String>,
    log: Mutex<Vec<Call>>,
}

impl RecordingCache {
    pub(crate) fn calls(&self) -> Vec<Call> {
        self.log.lock().unwrap().clone()
    }

    // the call is logged first, so a failed call still shows in the order
    fn answer(&self, call: Call) -> Result<(), UsageCacheError> {
        let (method, key) = match &call {
            Call::Get { key, .. } => ("get", key.as_str()),
            Call::Set { key, .. } => ("set", key.as_str()),
            Call::Increment { key, .. } => ("increment", key.as_str()),
            Call::BatchGet { .. } => ("batch_get", "async_batch_get_cache"),
            Call::BatchSet { .. } => ("batch_set", BATCH_WRITE),
        };
        let one_method = format!("{method} {key}");
        let fails = self
            .failing
            .iter()
            .any(|failing| *failing == key || *failing == one_method);
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        if fails {
            return Err(UsageCacheError::Transport(
                DualCacheError::UnrepresentableValue,
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl CounterCache for RecordingCache {
    async fn get(
        &self,
        key: &str,
        _parent_otel_span: Option<&ParentOtelSpan>,
        local_only: bool,
    ) -> Result<Option<LegacyCacheValue>, UsageCacheError> {
        let read = Call::Get {
            key: key.to_owned(),
            local_only,
        };
        self.answer(read).map(|()| {
            self.stored
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(key)
                .copied()
        })
    }

    async fn set(
        &self,
        key: &str,
        value: LegacyCacheValue,
        ttl: u64,
        _parent_otel_span: Option<&ParentOtelSpan>,
    ) -> Result<(), UsageCacheError> {
        let stored_key = key.to_owned();
        let write = Call::Set {
            key: stored_key.clone(),
            value,
            ttl,
        };
        self.answer(write)?;
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        stored.insert(stored_key, value);
        drop(stored);
        Ok(())
    }

    async fn increment(
        &self,
        key: &str,
        delta: i64,
        _parent_otel_span: Option<&ParentOtelSpan>,
        local_only: bool,
    ) -> Result<(), UsageCacheError> {
        self.answer(Call::Increment {
            key: key.to_owned(),
            delta,
            local_only,
        })?;
        let mut stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        let count = match stored.get(key) {
            Some(LegacyCacheValue::Count(current)) => current.saturating_add(delta),
            None => delta,
            // DualCache's increment fails on a stored object
            Some(LegacyCacheValue::Counter(_)) => {
                return Err(UsageCacheError::Transport(
                    DualCacheError::UnrepresentableValue,
                ));
            }
        };
        stored.insert(key.to_owned(), LegacyCacheValue::Count(count));
        drop(stored);
        Ok(())
    }
}

#[async_trait]
impl LegacyUsageCache for RecordingCache {
    async fn batch_get(
        &self,
        keys: &[Option<String>],
        _parent_otel_span: Option<&ParentOtelSpan>,
    ) -> Result<Option<Vec<Option<LegacyCacheValue>>>, UsageCacheError> {
        self.answer(Call::BatchGet {
            keys: keys.to_vec(),
        })?;
        Ok(None)
    }

    async fn batch_set(
        &self,
        entries: &[(String, LegacyCacheValue)],
        ttl: u64,
        _parent_otel_span: Option<&ParentOtelSpan>,
        local_only: bool,
    ) -> Result<(), UsageCacheError> {
        // nothing reads the counters back after the success hook's one batch write
        self.answer(Call::BatchSet {
            entries: entries.to_vec(),
            ttl,
            local_only,
        })
    }
}

struct PinnedClock;

impl LegacyClock for PinnedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-11-21T09:05:30+01:00").unwrap()
    }
}

pub(crate) struct Console {
    lines: Mutex<Vec<String>>,
}

impl Console {
    pub(crate) fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

impl VerboseConsole for Console {
    fn print_line(&self, statement: &str) -> bool {
        self.lines.lock().unwrap().push(statement.to_owned());
        true
    }
}

struct NoProviders;

impl ProviderResolution for NoProviders {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}

pub(crate) struct Fixture {
    pub(crate) cache: Arc<RecordingCache>,
    pub(crate) console: Arc<Console>,
    pub(crate) handler: LegacyRateLimitHandler,
}

pub(crate) fn fixture(stored: &[(&str, LegacyCacheValue)], failing: &[&str]) -> Fixture {
    let cache = Arc::new(RecordingCache {
        stored: Mutex::new(
            stored
                .iter()
                .map(|(key, value)| ((*key).to_owned(), *value))
                .collect(),
        ),
        failing: failing.iter().map(|key| (*key).to_owned()).collect(),
        log: Mutex::new(Vec::new()),
    });
    let console = Arc::new(Console {
        lines: Mutex::new(Vec::new()),
    });
    let usage_cache: Arc<dyn LegacyUsageCache> = Arc::<RecordingCache>::clone(&cache);
    let verbose: Arc<dyn VerboseConsole> = Arc::<Console>::clone(&console);
    let handler = LegacyRateLimitHandler {
        internal_usage_cache: usage_cache,
        clock: Arc::new(PinnedClock),
        verbose,
        set_verbose: true,
        native: Arc::new(NoProviders),
        router: None,
    };
    Fixture {
        cache,
        console,
        handler,
    }
}

pub(crate) const fn counter(requests: i64, tpm: i64, rpm: i64) -> LegacyCacheValue {
    LegacyCacheValue::Counter(MinuteCounter {
        current_requests: requests,
        current_tpm: tpm,
        current_rpm: rpm,
    })
}

pub(crate) fn minute_key(id: &str) -> String {
    format!("{id}::{MINUTE}::request_count")
}

struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedBuffer {
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

// the tracing output of one test thread, read back as text
pub(crate) struct Logs {
    buffer: Arc<Mutex<Vec<u8>>>,
    _guard: DefaultGuard,
}

impl Logs {
    pub(crate) fn capture() -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer_buffer = Arc::clone(&buffer);
        let subscriber = fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(Level::DEBUG)
            .with_writer(move || SharedBuffer(Arc::clone(&writer_buffer)))
            .finish();
        Self {
            buffer,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.buffer.lock().unwrap().clone()).unwrap()
    }
}
