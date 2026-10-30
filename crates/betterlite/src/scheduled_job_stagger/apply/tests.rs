#![cfg(test)]

use std::collections::BTreeMap;
use std::mem;
use std::time::Duration;

use betterlite_config::sections::scheduled_job_stagger::Settings;
use chrono::{DateTime, Duration as ChronoDuration, Utc};

use super::{OffsetTrigger, apply_scheduled_job_stagger, offset_for, stagger_trigger};
use crate::scheduled_job_stagger::{JobScheduler, JobSnapshot, SubmissionListener, Trigger};

struct FixedTrigger(DateTime<Utc>);

impl Trigger for FixedTrigger {
    fn next_fire_time(
        &self,
        _previous_fire_time: Option<DateTime<Utc>>,
        _now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        Some(self.0)
    }
}

struct NeverFires;

impl Trigger for NeverFires {
    fn next_fire_time(
        &self,
        _previous_fire_time: Option<DateTime<Utc>>,
        _now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        None
    }
}

struct EchoNow;

impl Trigger for EchoNow {
    fn next_fire_time(
        &self,
        _previous_fire_time: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        Some(now)
    }
}

#[test]
fn offset_trigger_shifts_the_base_triggers_answer_forward() {
    let base = Utc::now();
    let wrapper = OffsetTrigger {
        base: Box::new(FixedTrigger(base)),
        offset: Duration::from_secs(30),
    };
    let fired = wrapper
        .next_fire_time(None, Utc::now())
        .expect("should fire");
    assert_eq!(fired, base + ChronoDuration::seconds(30));
}

#[test]
fn offset_trigger_propagates_a_base_trigger_that_never_fires() {
    let wrapper = OffsetTrigger {
        base: Box::new(NeverFires),
        offset: Duration::from_secs(30),
    };
    assert_eq!(wrapper.next_fire_time(None, Utc::now()), None);
}

#[test]
fn offset_trigger_shifts_now_back_before_delegating_and_forward_after() {
    let now = Utc::now();
    let wrapper = OffsetTrigger {
        base: Box::new(EchoNow),
        offset: Duration::from_secs(90),
    };
    assert_eq!(wrapper.next_fire_time(None, now), Some(now));
}

#[test]
fn offset_for_uses_a_clamped_override_when_one_is_configured() {
    let mut offsets = BTreeMap::new();
    offsets.insert("ptu_flat_cost_rollup_job".to_owned(), 5000_i64);
    let settings = Settings {
        enabled: true,
        window_seconds: 300,
        identity: None,
        offsets,
    };
    assert_eq!(
        offset_for(
            "ptu_flat_cost_rollup_job",
            None,
            true,
            &settings,
            "identity"
        ),
        899
    );
}

#[test]
fn offset_for_is_zero_for_a_non_staggerable_job_without_an_override() {
    let settings = Settings::default();
    assert_eq!(
        offset_for("some_job", None, false, &settings, "identity"),
        0
    );
}

#[test]
fn offset_for_ignores_an_override_for_a_non_staggerable_job() {
    let mut offsets = BTreeMap::new();
    offsets.insert("anchored-job".to_owned(), 5_i64);
    let settings = Settings {
        enabled: true,
        window_seconds: 300,
        identity: None,
        offsets,
    };
    assert_eq!(
        offset_for("anchored-job", None, false, &settings, "identity"),
        0
    );
}

#[test]
fn stagger_trigger_wraps_when_an_override_offset_applies() {
    let mut offsets = BTreeMap::new();
    offsets.insert("my_job".to_owned(), 5_i64);
    let settings = Settings {
        enabled: true,
        window_seconds: 300,
        identity: None,
        offsets,
    };
    let base = Utc::now();
    let wrapped = stagger_trigger(
        "my_job",
        Box::new(FixedTrigger(base)),
        None,
        &settings,
        "identity",
    );
    let fired = wrapped
        .next_fire_time(None, Utc::now())
        .expect("should fire");
    assert_eq!(fired, base + ChronoDuration::seconds(5));
}

struct FakeScheduler {
    running: bool,
    jobs: Vec<JobSnapshot>,
    modified: Vec<String>,
}

impl JobScheduler for FakeScheduler {
    fn running(&self) -> bool {
        self.running
    }

    fn jobs(&mut self) -> Vec<JobSnapshot> {
        mem::take(&mut self.jobs)
    }

    fn modify_job(&mut self, job_id: &str, _trigger: Box<dyn Trigger>) {
        self.modified.push(job_id.to_owned());
    }

    fn add_submission_listener(&mut self, _listener: SubmissionListener) {}
}

fn interval_job(id: &str) -> JobSnapshot {
    JobSnapshot {
        id: id.to_owned(),
        next_run_time: None,
        period_seconds: Some(60),
        trigger: Box::new(NeverFires),
    }
}

#[test]
fn apply_skips_every_job_when_the_scheduler_is_already_running() {
    let mut scheduler = FakeScheduler {
        running: true,
        jobs: vec![interval_job("job-a")],
        modified: vec![],
    };
    let offsets = apply_scheduled_job_stagger(
        &mut scheduler,
        &Settings::default(),
        Some("identity"),
        &BTreeMap::new(),
    );
    assert_eq!(offsets.get("job-a"), Some(&0));
    assert!(scheduler.modified.is_empty());
}

#[test]
fn apply_skips_every_job_when_staggering_is_disabled() {
    let settings = Settings {
        enabled: false,
        window_seconds: 300,
        identity: None,
        offsets: BTreeMap::new(),
    };
    let mut scheduler = FakeScheduler {
        running: false,
        jobs: vec![interval_job("job-a")],
        modified: vec![],
    };
    let offsets = apply_scheduled_job_stagger(
        &mut scheduler,
        &settings,
        Some("identity"),
        &BTreeMap::new(),
    );
    assert_eq!(offsets.get("job-a"), Some(&0));
    assert!(scheduler.modified.is_empty());
}

#[test]
fn apply_modifies_every_job_an_override_gives_a_nonzero_offset_to() {
    let mut offset_overrides = BTreeMap::new();
    offset_overrides.insert("job-a".to_owned(), 5_i64);
    let settings = Settings {
        enabled: true,
        window_seconds: 300,
        identity: None,
        offsets: offset_overrides,
    };
    let mut scheduler = FakeScheduler {
        running: false,
        jobs: vec![interval_job("job-a")],
        modified: vec![],
    };
    let offsets = apply_scheduled_job_stagger(
        &mut scheduler,
        &settings,
        Some("identity"),
        &BTreeMap::new(),
    );
    assert_eq!(offsets.get("job-a"), Some(&5));
    assert_eq!(scheduler.modified, vec!["job-a".to_owned()]);
}

#[test]
fn apply_does_not_modify_a_job_when_its_offset_is_zero() {
    let mut offset_overrides = BTreeMap::new();
    offset_overrides.insert("job-a".to_owned(), 0_i64);
    let settings = Settings {
        enabled: true,
        window_seconds: 300,
        identity: None,
        offsets: offset_overrides,
    };
    let mut scheduler = FakeScheduler {
        running: false,
        jobs: vec![interval_job("job-a")],
        modified: vec![],
    };
    let offsets = apply_scheduled_job_stagger(
        &mut scheduler,
        &settings,
        Some("identity"),
        &BTreeMap::new(),
    );
    assert_eq!(offsets.get("job-a"), Some(&0));
    assert!(scheduler.modified.is_empty());
}
