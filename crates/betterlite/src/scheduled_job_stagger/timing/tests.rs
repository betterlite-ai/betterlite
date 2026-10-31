#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tracing::subscriber::with_default;

use super::attach_job_timing_logger;
use crate::scheduled_job_stagger::{
    JobScheduler, JobSnapshot, JobSubmission, SubmissionListener, Trigger,
};

struct FakeScheduler {
    listener: Option<SubmissionListener>,
}

impl JobScheduler for FakeScheduler {
    fn running(&self) -> bool {
        false
    }

    fn jobs(&mut self) -> Vec<JobSnapshot> {
        vec![]
    }

    fn modify_job(&mut self, _job_id: &str, _trigger: Box<dyn Trigger>) {}

    fn add_submission_listener(&mut self, listener: SubmissionListener) {
        self.listener = Some(listener);
    }
}

struct FakeSubmission {
    job_id: String,
    scheduled_run_times: Vec<DateTime<Utc>>,
}

impl JobSubmission for FakeSubmission {
    fn job_id(&self) -> &str {
        &self.job_id
    }

    fn scheduled_run_times(&self) -> &[DateTime<Utc>] {
        &self.scheduled_run_times
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn submit_with_capture(event: &FakeSubmission, now: DateTime<Utc>) -> String {
    let mut scheduler = FakeScheduler { listener: None };
    attach_job_timing_logger(&mut scheduler, move || now);
    let listener = scheduler
        .listener
        .expect("a listener should have been attached");
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    with_default(subscriber, || listener(event));
    String::from_utf8(
        captured
            .0
            .lock()
            .expect("capture mutex should not be poisoned")
            .clone(),
    )
    .expect("captured log output should be UTF-8")
}

#[test]
fn logs_the_scheduled_and_actual_start_time() {
    let now = Utc::now();
    let event = FakeSubmission {
        job_id: "job-a".to_owned(),
        scheduled_run_times: vec![now],
    };
    let output = submit_with_capture(&event, now);
    assert!(output.contains("job-a"));
    assert!(output.contains("scheduled job started"));
}

#[test]
fn does_nothing_when_no_run_times_were_scheduled() {
    let event = FakeSubmission {
        job_id: "job-a".to_owned(),
        scheduled_run_times: vec![],
    };
    let output = submit_with_capture(&event, Utc::now());
    assert!(output.is_empty());
}

#[test]
fn logs_the_exact_delay_between_the_scheduled_and_actual_start_time() {
    let scheduled = Utc::now();
    let started = scheduled + ChronoDuration::milliseconds(2500);
    let event = FakeSubmission {
        job_id: "job-a".to_owned(),
        scheduled_run_times: vec![scheduled],
    };
    let output = submit_with_capture(&event, started);
    assert!(output.contains("delay_milliseconds=2500"));
}
