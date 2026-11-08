use chrono::{DateTime, Utc};
use serde::Deserialize;

use betterlite::job_scheduler::triggers::{CronTrigger, CronZone, IntervalTrigger};
use betterlite::scheduled_job_stagger::Trigger;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    cron: Option<String>,
    #[serde(default)]
    interval: Option<u64>,
    start: DateTime<Utc>,
}

pub(crate) struct Op;

const FIRE_COUNT: usize = 3;

fn fires(trigger: &dyn Trigger, start: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let mut times = Vec::new();
    let mut previous: Option<DateTime<Utc>> = None;
    while times.len() < FIRE_COUNT {
        let now = previous.unwrap_or(start);
        let Some(next) = trigger.next_fire_time(previous, now) else {
            break;
        };
        times.push(next);
        previous = Some(next);
    }
    times
}

fn fire_times(trigger: &dyn Trigger, start: DateTime<Utc>) -> serde_json::Value {
    let times: Vec<String> = fires(trigger, start)
        .iter()
        .map(DateTime::to_rfc3339)
        .collect();
    serde_json::json!({ "fires": times })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match (&input.cron, input.interval) {
            (Some(cron), None) => match CronTrigger::from_crontab(cron, CronZone::Utc) {
                Ok(trigger) => Ok(fire_times(&trigger, input.start)),
                Err(error) => Ok(serde_json::json!({ "error": error.to_string() })),
            },
            (None, Some(seconds)) => {
                let trigger = IntervalTrigger { seconds };
                Ok(fire_times(&trigger, input.start))
            }
            _ => Err(Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "give exactly one of cron or interval".to_owned(),
            })),
        }
    }
}
