#![no_main]

use std::str;

use chrono::DateTime;
use libfuzzer_sys::fuzz_target;

use betterlite::job_scheduler::triggers::{CronTrigger, CronZone};
use betterlite::scheduled_job_stagger::Trigger as _;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = str::from_utf8(data) else {
        return;
    };
    let Ok(trigger) = CronTrigger::from_crontab(text, CronZone::Utc) else {
        return;
    };
    let _next = trigger.next_fire_time(None, DateTime::UNIX_EPOCH);
});
