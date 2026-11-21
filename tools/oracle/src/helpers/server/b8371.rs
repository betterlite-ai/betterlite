use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite::cleanup_schedule::SPEND_LOG_CLEANUP_JOB_ID;
use betterlite::cleanup_schedule::changes::{
    CleanupScheduleState, ScheduleChange, on_settings_changed,
};
use betterlite::cleanup_schedule::ports::CleanupScheduler;
use betterlite::cleanup_schedule::schedule::cleanup_schedule_of;
use betterlite::cleanup_schedule::trigger::{ScheduleContext, cleanup_trigger};
use betterlite::job_scheduler::{BoxFuture, JobFn, JobSpec, SchedulerError, SchedulerState};
use betterlite::scheduled_job_stagger::Trigger;
use betterlite_config::sections::scheduled_job_stagger::Settings as StaggerSettings;
use betterlite_config::sections::settings::{JsonValue, Section, SettingsStore};
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;

const RETENTION_INTERVAL_KEY: &str = "maximum_spend_logs_retention_interval";

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    CleanupTrigger {
        settings: BTreeMap<String, JsonValue>,
        start: DateTime<Utc>,
    },
    ApplyRetention {
        settings: BTreeMap<String, JsonValue>,
        previous: BTreeMap<String, JsonValue>,
        #[serde(default)]
        last_attempt: Option<BTreeMap<String, JsonValue>>,
        #[serde(default)]
        reschedule_failed: bool,
        scheduler: SchedulerChoice,
        #[serde(default)]
        job_present: bool,
        #[serde(default)]
        add_fails: bool,
        start: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SchedulerChoice {
    None,
    Stopped,
    Paused,
}

#[derive(Debug, Clone, Copy)]
enum JobPresence {
    Present,
    Absent,
}

impl From<bool> for JobPresence {
    fn from(present: bool) -> Self {
        if present { Self::Present } else { Self::Absent }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::CleanupTrigger { settings, start } => trigger_outcome(settings, *start),
            Input::ApplyRetention {
                settings,
                previous,
                last_attempt,
                reschedule_failed,
                scheduler,
                job_present,
                add_fails,
                start,
            } => retention_outcome(&RetentionSetup {
                settings,
                previous,
                last_attempt: last_attempt.as_ref(),
                reschedule_failed: *reschedule_failed,
                scheduler: *scheduler,
                presence: JobPresence::from(*job_present),
                add_fails: *add_fails,
                start: *start,
            }),
        }
    }
}

struct RetentionSetup<'a> {
    settings: &'a BTreeMap<String, JsonValue>,
    previous: &'a BTreeMap<String, JsonValue>,
    last_attempt: Option<&'a BTreeMap<String, JsonValue>>,
    reschedule_failed: bool,
    scheduler: SchedulerChoice,
    presence: JobPresence,
    add_fails: bool,
    start: DateTime<Utc>,
}

struct ScheduleFixture {
    stagger: StaggerSettings,
    database: TimeZoneDatabase,
    now: DateTime<Utc>,
}

impl ScheduleFixture {
    fn context(&self) -> ScheduleContext<'_> {
        ScheduleContext {
            stagger: &self.stagger,
            identity: "oracle",
            now: self.now,
            timezone: None,
            database: &self.database,
        }
    }
}

fn trigger_outcome(values: &BTreeMap<String, JsonValue>, start: DateTime<Utc>) -> HelperResult {
    let fixture = ScheduleFixture {
        stagger: StaggerSettings::default(),
        database: TimeZoneDatabase::from_env(),
        now: start,
    };
    let context = fixture.context();
    let settings = store(values);
    let trigger = cleanup_trigger(&settings, &context);
    let interval = interval_seconds(&settings, &context);
    let (kind, gap) = classification(trigger.as_deref(), interval, context.now);
    Ok(serde_json::json!({ "kind": kind, "gap_seconds": gap }))
}

fn retention_outcome(setup: &RetentionSetup<'_>) -> HelperResult {
    let fixture = ScheduleFixture {
        stagger: StaggerSettings::default(),
        database: TimeZoneDatabase::from_env(),
        now: setup.start,
    };
    let context = fixture.context();
    let settings = store(setup.settings);
    let previous = cleanup_schedule_of(&store(setup.previous));
    let mut state = CleanupScheduleState {
        last_attempt: setup
            .last_attempt
            .map(|values| cleanup_schedule_of(&store(values))),
        reschedule_failed: setup.reschedule_failed,
    };
    let mut recording = recording_scheduler(setup);
    let job: JobFn = Arc::new(|| {
        let future: BoxFuture<'static, ()> = Box::pin(async {});
        future
    });
    on_settings_changed(
        scheduler_option(setup.scheduler, &mut recording),
        &ScheduleChange {
            previous: &previous,
            context: &context,
        },
        &settings,
        job,
        &mut state,
    );
    let interval = interval_seconds(&settings, &context);
    let (kind, gap) = classification(recording.trigger.as_deref(), interval, context.now);
    Ok(serde_json::json!({
        "added": recording.added,
        "removed": recording.removed,
        "job_present": recording.job_present,
        "kind": kind,
        "gap_seconds": gap,
    }))
}

fn recording_scheduler(setup: &RetentionSetup<'_>) -> RecordingScheduler {
    let presence = match setup.scheduler {
        SchedulerChoice::None => JobPresence::Absent,
        SchedulerChoice::Stopped | SchedulerChoice::Paused => setup.presence,
    };
    RecordingScheduler {
        state: match setup.scheduler {
            SchedulerChoice::None | SchedulerChoice::Paused => SchedulerState::Paused,
            SchedulerChoice::Stopped => SchedulerState::Stopped,
        },
        job_present: matches!(presence, JobPresence::Present),
        add_fails: setup.add_fails,
        added: 0,
        removed: 0,
        trigger: None,
    }
}

fn scheduler_option(
    choice: SchedulerChoice,
    recording: &mut RecordingScheduler,
) -> Option<&mut dyn CleanupScheduler> {
    match choice {
        SchedulerChoice::None => None,
        SchedulerChoice::Stopped | SchedulerChoice::Paused => Some(recording),
    }
}

// The trigger trait exposes only fire times, so the interval schedule is the one whose fire gap is the configured period.
fn classification(
    trigger: Option<&dyn Trigger>,
    interval: Option<u64>,
    now: DateTime<Utc>,
) -> (Option<&'static str>, Option<u64>) {
    let Some(trigger) = trigger else {
        return (None, None);
    };
    let gap = fire_gap(trigger, now);
    let kind = if gap.is_some() && gap == interval {
        "interval"
    } else {
        "cron"
    };
    (Some(kind), gap)
}

fn fire_gap(trigger: &dyn Trigger, now: DateTime<Utc>) -> Option<u64> {
    let first = trigger.next_fire_time(None, now)?;
    let second = trigger.next_fire_time(Some(first), first)?;
    u64::try_from(second.signed_duration_since(first).num_seconds()).ok()
}

fn interval_seconds(settings: &SettingsStore, context: &ScheduleContext<'_>) -> Option<u64> {
    let schedule = cleanup_schedule_of(settings);
    let text = match schedule.cleanup_interval.as_ref() {
        None => "1d",
        Some(JsonValue::String(text)) => text.as_str(),
        Some(_) => return None,
    };
    let parsed = duration_in_seconds(text, context.now, context.timezone, context.database).ok()?;
    u64::try_from(parsed.seconds).ok()
}

fn store(values: &BTreeMap<String, JsonValue>) -> SettingsStore {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(values.clone());
    store
}

struct RecordingScheduler {
    state: SchedulerState,
    job_present: bool,
    add_fails: bool,
    added: u32,
    removed: u32,
    trigger: Option<Box<dyn Trigger>>,
}

impl CleanupScheduler for RecordingScheduler {
    fn scheduler_state(&self) -> SchedulerState {
        self.state
    }

    fn has_job(&self, id: &str) -> bool {
        id == SPEND_LOG_CLEANUP_JOB_ID && self.job_present
    }

    fn insert_job(&mut self, spec: JobSpec) -> Result<(), SchedulerError> {
        if self.add_fails {
            return Err(SchedulerError::ConflictingId { id: spec.id });
        }
        self.added = self.added.saturating_add(1);
        self.trigger = Some(spec.trigger);
        self.job_present = true;
        Ok(())
    }

    fn remove_existing_job(&mut self, id: &str) -> Result<(), SchedulerError> {
        if !self.job_present {
            return Err(SchedulerError::JobLookup { id: id.to_owned() });
        }
        self.removed = self.removed.saturating_add(1);
        self.job_present = false;
        Ok(())
    }
}
