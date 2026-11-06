#![cfg(test)]

mod duration_parser_duration_tests_behavior;

use std::fs;
use std::path::PathBuf;
use std::str;

use chrono::{DateTime, NaiveTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::duration::reset::next_standardized_reset_time;
use crate::duration::seconds::duration_in_seconds;

fn replay_spec(spec: &str, database: &TimeZoneDatabase) {
    let moments = [
        "2024-03-10T06:30:00Z",
        "2024-11-03T06:00:00Z",
        "2024-02-29T23:59:59Z",
        "2024-12-31T12:00:00Z",
    ]
    .map(|text| {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    });
    let times = ["00:00:00", "01:30:00", "02:30:00"]
        .map(|text| NaiveTime::parse_from_str(text, "%H:%M:%S%.f").unwrap());
    for moment in moments {
        for zone in [None, Some("America/New_York"), Some("Asia/Kolkata")] {
            drop(duration_in_seconds(spec, moment, zone, database));
            for time_of_day in times {
                drop(next_standardized_reset_time(
                    spec,
                    moment,
                    zone,
                    time_of_day,
                    database,
                ));
            }
        }
    }
}

#[rstest::rstest]
fn replays_the_duration_spec_corpus(#[files("fuzz/corpus/duration_spec/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(spec) = str::from_utf8(&bytes) else {
        return;
    };
    let database = TimeZoneDatabase::from_env();
    replay_spec(spec, &database);
}
