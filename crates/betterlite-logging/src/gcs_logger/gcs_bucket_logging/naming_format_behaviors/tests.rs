#![cfg(test)]

use chrono::{TimeZone as _, Utc};

use super::{
    generate_batch_object_name, generate_failure_object_name, generate_success_object_name,
    get_object_date,
};

#[test]
fn the_success_name_is_the_date_and_response_id() {
    assert_eq!(
        generate_success_object_name("2024-03-09", "chatcmpl-abc"),
        "2024-03-09/chatcmpl-abc"
    );
}

#[test]
fn the_failure_name_is_the_date_failure_prefix_and_uuid() {
    assert_eq!(
        generate_failure_object_name("2024-03-09", "0123456789abcdef"),
        "2024-03-09/failure-0123456789abcdef"
    );
}

#[test]
fn the_batch_name_is_the_date_batch_prefix_and_ndjson_suffix() {
    assert_eq!(
        generate_batch_object_name("2024-03-09", "1704067199000-00000000"),
        "2024-03-09/batch-1704067199000-00000000.ndjson"
    );
}

#[test]
fn the_object_date_is_the_utc_calendar_date() {
    let instant = Utc
        .with_ymd_and_hms(2024, 1, 2, 0, 0, 0)
        .single()
        .expect("the instant is valid");
    assert_eq!(get_object_date(instant), "2024-01-02");
    let late = Utc
        .with_ymd_and_hms(2024, 3, 9, 23, 59, 59)
        .single()
        .expect("the instant is valid");
    assert_eq!(get_object_date(late), "2024-03-09");
    let next = Utc
        .with_ymd_and_hms(2024, 3, 10, 0, 0, 0)
        .single()
        .expect("the instant is valid");
    assert_eq!(get_object_date(next), "2024-03-10");
}
