#![no_main]

use std::str;

use betterlite_core::duration::reset::next_standardized_reset_time;
use betterlite_core::duration::seconds::duration_in_seconds;
use chrono::{DateTime, NaiveTime, Utc};
use jiff::tz::TimeZoneDatabase;
use libfuzzer_sys::fuzz_target;

const MOMENTS: [&str; 4] = [
    "2024-03-10T06:30:00Z",
    "2024-11-03T06:00:00Z",
    "2024-02-29T23:59:59Z",
    "2024-12-31T12:00:00Z",
];

const TIMES: [&str; 3] = ["00:00:00", "01:30:00", "02:30:00"];

const ZONES: [Option<&str>; 3] = [None, Some("America/New_York"), Some("Asia/Kolkata")];

fuzz_target!(|bytes: &[u8]| {
    let Ok(spec) = str::from_utf8(bytes) else {
        return;
    };
    let database = TimeZoneDatabase::from_env();
    for moment_text in MOMENTS {
        let Ok(zone_time) = DateTime::parse_from_rfc3339(moment_text) else {
            continue;
        };
        let moment = zone_time.with_timezone(&Utc);
        for zone in ZONES {
            drop(duration_in_seconds(spec, moment, zone, &database));
            for time_text in TIMES {
                let Ok(time_of_day) = NaiveTime::parse_from_str(time_text, "%H:%M:%S%.f") else {
                    continue;
                };
                drop(next_standardized_reset_time(
                    spec,
                    moment,
                    zone,
                    time_of_day,
                    &database,
                ));
            }
        }
    }
});
