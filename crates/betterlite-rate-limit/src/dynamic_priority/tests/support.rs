use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use crate::dynamic_priority::cache::SaturationCache;
use crate::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};
use crate::limiter_records::{CacheCounterValue, LimiterClock};
use crate::provider_attribution::{PriorityKind, PriorityReservation, PriorityValue};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CacheRead {
    pub(crate) key: String,
    pub(crate) local_only: bool,
    pub(crate) local_ttl: i64,
}

// the usage cache the saturation check reads: seeded values, and every read recorded with the flags it got
#[derive(Default)]
pub(crate) struct RecordingCache {
    values: BTreeMap<String, CacheCounterValue>,
    reads: Mutex<Vec<CacheRead>>,
}

impl RecordingCache {
    pub(crate) fn seeded(values: &[(&str, CacheCounterValue)]) -> Self {
        Self {
            values: values
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<CacheRead> {
        let reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        reads.clone()
    }
}

#[async_trait::async_trait]
impl SaturationCache for RecordingCache {
    async fn get_cache(
        &self,
        key: &str,
        local_only: bool,
        local_ttl: i64,
    ) -> Option<CacheCounterValue> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        reads.push(CacheRead {
            key: key.to_owned(),
            local_only,
            local_ttl,
        });
        drop(reads);
        self.values.get(key).cloned()
    }
}

pub(crate) struct FixedClock;

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .expect("the fixed clock stays representable")
    }
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut output) = self.0.lock() else {
            return Err(io::Error::other("poisoned capture"));
        };
        output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn capture<F>(log: F) -> String
where
    F: FnOnce(),
{
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&output);
    let subscriber = fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(Level::TRACE)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    with_default(subscriber, log);
    let bytes = output.lock().expect("the capture is not poisoned").clone();
    String::from_utf8(bytes).expect("the capture is UTF-8")
}

pub(crate) fn reservation_map(
    entries: &[(&str, PriorityValue)],
) -> BTreeMap<String, PriorityValue> {
    entries
        .iter()
        .map(|(priority, value)| ((*priority).to_owned(), *value))
        .collect()
}

pub(crate) const fn percent(value: f64) -> PriorityValue {
    PriorityValue::Percent(value)
}

pub(crate) const fn reserved(kind: PriorityKind, value: f64) -> PriorityValue {
    PriorityValue::Reservation(PriorityReservation { kind, value })
}

pub(crate) fn policy(reservation: Option<&[(&str, PriorityValue)]>) -> PriorityPolicy {
    PriorityPolicy {
        settings: PrioritySettings::default(),
        reservation: reservation.map(reservation_map),
    }
}
