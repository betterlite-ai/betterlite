#![cfg(test)]

use chrono::{DateTime, TimeDelta, Utc};

use crate::scheduled_job_stagger::Trigger;

use super::due_run_times;

struct Stalled;

impl Trigger for Stalled {
    fn next_fire_time(
        &self,
        previous_fire_time: Option<DateTime<Utc>>,
        _now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        previous_fire_time
    }
}

#[test]
fn a_trigger_that_does_not_advance_stops_its_due_runs() {
    let start = DateTime::from_timestamp(1_767_225_600, 0).unwrap();
    let now = start + TimeDelta::seconds(5);
    assert_eq!(due_run_times(&Stalled, start, now), [start]);
}
