use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::CacheValue;
use tracing::Subscriber;

use crate::backend::memory::MemoryCacheOps;
use crate::backend::{BackendError, CacheResult, Ttl};

#[path = "../../dual_cache/tests/support.rs"]
pub(crate) mod redis;

// a memory tier that can fail its batch read or its backfill write the way a broken backend would
#[derive(Default)]
pub(crate) struct FailingMemory {
    pub(crate) values: Mutex<BTreeMap<String, CacheValue>>,
    pub(crate) fail_reads: bool,
    pub(crate) fail_writes: bool,
}

impl MemoryCacheOps for FailingMemory {
    fn default_ttl(&self) -> i64 {
        redis::DEFAULT_TTL_SECONDS
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        if self.fail_reads {
            return Err(BackendError::Unsupported("get_cache"));
        }
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(values.get(key).cloned())
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        if self.fail_writes {
            return Err(BackendError::Unsupported("set_cache"));
        }
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.insert(key.to_owned(), value);
        Ok(())
    }

    fn delete_cache(&self, _key: &str) -> CacheResult<()> {
        Err(BackendError::Unsupported("delete_cache"))
    }

    fn flush_cache(&self) -> CacheResult<()> {
        Err(BackendError::Unsupported("flush_cache"))
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        if self.fail_reads {
            return Err(BackendError::Unsupported("batch_get_cache"));
        }
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(keys.iter().map(|key| values.get(key).cloned()).collect())
    }

    fn increment_cache(&self, _key: &str, _value: f64, _ttl: Ttl) -> CacheResult<f64> {
        Err(BackendError::Unsupported("increment_cache"))
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        Err(BackendError::Unsupported("ttl_entries"))
    }
}

struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
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

pub(crate) fn capture_logs() -> (impl Subscriber + Send + Sync, Arc<Mutex<Vec<u8>>>) {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&bytes);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || Capture(Arc::clone(&sink)))
        .finish();
    (subscriber, bytes)
}

pub(crate) fn captured(bytes: &Mutex<Vec<u8>>) -> String {
    let log = bytes.lock().unwrap_or_else(PoisonError::into_inner).clone();
    String::from_utf8(log).expect("the log is utf-8")
}
