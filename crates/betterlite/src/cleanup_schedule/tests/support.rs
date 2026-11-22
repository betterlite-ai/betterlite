use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::scheduled_job_stagger::Settings as StaggerSettings;
use betterlite_config::sections::settings::{JsonValue, Section, SettingsStore};
use chrono::{DateTime, TimeDelta, Utc};
use jiff::tz::TimeZoneDatabase;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::cleanup_schedule::ports::CleanupScheduler;
use crate::cleanup_schedule::trigger::ScheduleContext;
use crate::cleanup_schedule::{RETENTION_PERIOD_KEY, SPEND_LOG_CLEANUP_JOB_ID};
use crate::job_scheduler::env::JobDefaults;
use crate::job_scheduler::triggers::IntervalTrigger;
use crate::job_scheduler::{
    BoxFuture, JobFn, JobSpec, ProxyScheduler, SchedulerError, SchedulerState, SystemClock,
};
use crate::scheduled_job_stagger::Trigger;

pub(super) fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_630, 0).unwrap()
}

pub(super) struct Fixture {
    stagger: StaggerSettings,
    database: TimeZoneDatabase,
}

pub(super) fn fixture() -> Fixture {
    Fixture {
        stagger: StaggerSettings::default(),
        database: TimeZoneDatabase::from_env(),
    }
}

impl Fixture {
    pub(super) fn context(&self) -> ScheduleContext<'_> {
        ScheduleContext {
            stagger: &self.stagger,
            identity: "test-identity",
            now: origin(),
            timezone: None,
            database: &self.database,
        }
    }
}

pub(super) fn store(pairs: &[(&str, JsonValue)]) -> SettingsStore {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    let values: BTreeMap<String, JsonValue> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect();
    store.load_yaml(values);
    store
}

pub(super) fn retained(period: &str) -> SettingsStore {
    store(&[(RETENTION_PERIOD_KEY, JsonValue::from(period))])
}

pub(super) fn noop_job() -> JobFn {
    Arc::new(|| {
        let future: BoxFuture<'static, ()> = Box::pin(async {});
        future
    })
}

pub(super) fn defaults() -> JobDefaults {
    JobDefaults {
        coalesce: true,
        misfire_grace_seconds: 3600,
        max_instances: 1,
    }
}

pub(super) fn proxy_scheduler() -> ProxyScheduler {
    ProxyScheduler::new(defaults(), Arc::new(SystemClock))
}

pub(super) struct FixedRng(pub(super) u64);

impl rand::TryRng for FixedRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(u32::try_from(self.0).unwrap_or(u32::MAX))
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        Ok(self.0)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        dst.fill(0);
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).unwrap()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_max_level(tracing::Level::TRACE)
        .without_time()
        .with_ansi(false)
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

pub(super) fn period_of(trigger: &dyn Trigger, now: DateTime<Utc>) -> Option<TimeDelta> {
    let first = trigger.next_fire_time(None, now)?;
    let second = trigger.next_fire_time(Some(first), first)?;
    Some(second.signed_duration_since(first))
}

pub(super) struct RecordingScheduler {
    pub(super) state: SchedulerState,
    pub(super) jobs: BTreeMap<String, Box<dyn Trigger>>,
    pub(super) inserted: usize,
    pub(super) removals: Vec<String>,
    pub(super) fail_insert: bool,
}

pub(super) fn recording(state: SchedulerState) -> RecordingScheduler {
    RecordingScheduler {
        state,
        jobs: BTreeMap::new(),
        inserted: 0,
        removals: Vec::new(),
        fail_insert: false,
    }
}

pub(super) fn with_job(mut scheduler: RecordingScheduler) -> RecordingScheduler {
    scheduler.jobs.insert(
        SPEND_LOG_CLEANUP_JOB_ID.to_owned(),
        Box::new(IntervalTrigger { seconds: 60 }),
    );
    scheduler
}

impl CleanupScheduler for RecordingScheduler {
    fn scheduler_state(&self) -> SchedulerState {
        self.state
    }

    fn has_job(&self, id: &str) -> bool {
        self.jobs.contains_key(id)
    }

    fn insert_job(&mut self, spec: JobSpec) -> Result<(), SchedulerError> {
        self.inserted = self.inserted.saturating_add(1);
        if self.fail_insert {
            return Err(SchedulerError::ConflictingId { id: spec.id });
        }
        self.jobs.insert(spec.id, spec.trigger);
        Ok(())
    }

    fn remove_existing_job(&mut self, id: &str) -> Result<(), SchedulerError> {
        self.removals.push(id.to_owned());
        if self.jobs.remove(id).is_some() {
            Ok(())
        } else {
            Err(SchedulerError::JobLookup { id: id.to_owned() })
        }
    }
}
