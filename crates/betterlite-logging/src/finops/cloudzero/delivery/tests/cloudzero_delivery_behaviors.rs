#![cfg(test)]

use std::error::Error as _;
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone as _, Utc};
use jiff::tz::{TimeZone, TimeZoneDatabase};
use rstest::rstest;
use rust_decimal::Decimal;

use super::fixtures::{
    API_KEY, CONNECTION_ID, FakeTransport, FixedClock, Refused, capture_logs, complete, posts_of,
    row, streamer, text_row,
};
use crate::finops::cloudzero::delivery::error::DeliveryError;
use crate::finops::cloudzero::delivery::payload::{convert_record, format_decimal};
use crate::finops::cloudzero::delivery::timestamp::{parse_timestamp, utc_iso};
use crate::finops::cloudzero::records::CbfValue;

#[test]
fn credentials_never_reach_debug_or_error_text() {
    let transport = FakeTransport {
        fail_on_call: Some(0),
        posted: Arc::default(),
    };
    let streamer = streamer(None, transport);
    let data = [text_row("2025-04-19T00:00:00Z")];
    let error = complete(streamer.send_batched(&data, None)).unwrap_err();
    let shown = format!(
        "{:?} {:?} {error} {error:?}",
        streamer.credentials, streamer.user_timezone
    );
    assert!(!shown.contains(API_KEY), "{shown}");
    assert!(!shown.contains(CONNECTION_ID), "{shown}");
}

#[test]
fn request_debug_hides_the_url_and_the_bearer_token() {
    let posts = posts_of(None, &[text_row("2025-04-19T00:00:00Z")], None);
    let shown = &posts[0].request_debug;
    assert!(shown.contains("timeout: 30s"), "{shown}");
    assert!(!shown.contains(API_KEY), "{shown}");
    assert!(!shown.contains(CONNECTION_ID), "{shown}");
}

#[rstest]
#[case::default_operation(None, "replace_hourly")]
#[case::replace_hourly(Some("replace_hourly"), "replace_hourly")]
#[case::sum(Some("sum"), "sum")]
#[case::unchanged(Some("replace_drop"), "replace_drop")]
fn operation_flows_unchanged_into_the_payload(
    #[case] operation: Option<&str>,
    #[case] expected: &str,
) {
    let posts = posts_of(None, &[text_row("2025-04-19T00:00:00Z")], operation);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].body.operation, expected);
}

#[rstest]
#[case::tokyo(Some("Asia/Tokyo"), "2025-04-18T15:00:00Z")]
#[case::unset(None, "2025-04-19T00:00:00Z")]
#[case::empty_name(Some(""), "2025-04-19T00:00:00Z")]
#[case::unknown_name(Some("Mars/Olympus_Mons"), "2025-04-19T00:00:00Z")]
#[case::path_like_name(Some("../etc/passwd"), "2025-04-19T00:00:00Z")]
fn timezone_resolution_falls_back_to_utc(
    #[case] user_timezone: Option<&str>,
    #[case] expected: &str,
) {
    let (posts, logs) =
        capture_logs(|| posts_of(user_timezone, &[text_row("2025-04-19T00:00:00")], None));
    assert_eq!(posts[0].body.data[0]["time/usage_start"], expected);
    let unknown = matches!(user_timezone, Some("Mars/Olympus_Mons" | "../etc/passwd"));
    assert_eq!(logs.contains("Unknown timezone"), unknown, "{logs}");
}

#[test]
fn empty_input_sends_nothing() {
    assert!(posts_of(None, &[], None).is_empty());
}

#[test]
fn rows_are_grouped_by_utc_date_and_posted_once_per_day() {
    let data = [
        text_row("2025-10-15T23:30:00-05:00"),
        text_row("2025-10-15T10:00:00Z"),
        text_row("2025-10-16T04:00:00+00:00"),
        text_row("2025-11-01T00:00:00Z"),
    ];
    let posts = posts_of(None, &data, None);
    let months: Vec<_> = posts.iter().map(|post| post.body.month.as_str()).collect();
    assert_eq!(months, ["2025-10", "2025-10", "2025-11"]);
    let sizes: Vec<_> = posts.iter().map(|post| post.body.data.len()).collect();
    assert_eq!(sizes, [2, 1, 1]);
    assert_eq!(
        posts[0].body.data[0]["time/usage_start"],
        "2025-10-16T04:30:00Z"
    );
    assert_eq!(
        posts[0].body.data[1]["time/usage_start"],
        "2025-10-16T04:00:00Z"
    );
}

#[test]
fn every_post_carries_the_endpoint_bearer_header_and_thirty_second_timeout() {
    let posts = posts_of(None, &[text_row("2025-10-15T10:00:00Z")], None);
    let post = &posts[0];
    assert_eq!(
        post.url,
        "https://api.cloudzero.com/v2/connections/billing/anycost/conn-5678/billing_drops"
    );
    assert_eq!(post.authorization, "Bearer cz-api-key-1234");
    assert_eq!(post.timeout, Duration::from_secs(30));
    assert_eq!(post.body.month, "2025-10");
    assert_eq!(post.body.operation, "replace_hourly");
    assert_eq!(
        post.body.data,
        [[
            ("time/usage_start", "2025-10-15T10:00:00Z"),
            ("cost/cost", "1.5"),
            ("usage/amount", "42"),
            ("resource/id", "czrn:x"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .into_iter()
        .collect()]
    );
}

#[rstest]
#[case::zulu("2025-04-19T10:00:00Z", "2025-04-19T10:00:00Z")]
#[case::positive_offset("2025-04-19T10:00:00+05:30", "2025-04-19T04:30:00Z")]
#[case::negative_offset("2025-04-19T10:00:00-05:00", "2025-04-19T15:00:00Z")]
#[case::utc_offset("2025-04-19T10:00:00+00:00", "2025-04-19T10:00:00Z")]
#[case::fraction("2025-04-19T10:00:00.250000Z", "2025-04-19T10:00:00.250000Z")]
#[case::space_separator("2025-04-19 10:00:00+02:00", "2025-04-19T08:00:00Z")]
fn timestamps_with_an_offset_ignore_the_user_timezone(#[case] text: &str, #[case] expected: &str) {
    let tokyo = TimeZoneDatabase::from_env().get("Asia/Tokyo").unwrap();
    assert_eq!(utc_iso(parse_timestamp(text, &tokyo).unwrap()), expected);
}

#[rstest]
#[case::tokyo_datetime("Asia/Tokyo", "2025-04-19T10:00:00", "2025-04-19T01:00:00Z")]
#[case::tokyo_date_only("Asia/Tokyo", "2025-04-19", "2025-04-18T15:00:00Z")]
#[case::new_york_summer("America/New_York", "2025-07-01T12:00:00", "2025-07-01T16:00:00Z")]
#[case::new_york_winter("America/New_York", "2025-01-01T12:00:00", "2025-01-01T17:00:00Z")]
#[case::utc("UTC", "2025-04-19T10:00:00", "2025-04-19T10:00:00Z")]
fn timestamps_without_an_offset_use_the_user_timezone(
    #[case] zone: &str,
    #[case] text: &str,
    #[case] expected: &str,
) {
    let user_timezone = TimeZoneDatabase::from_env().get(zone).unwrap();
    assert_eq!(
        utc_iso(parse_timestamp(text, &user_timezone).unwrap()),
        expected
    );
}

#[test]
fn a_local_evening_time_lands_on_the_next_utc_day() {
    let posts = posts_of(
        Some("America/New_York"),
        &[text_row("2025-04-19T22:00:00")],
        None,
    );
    assert_eq!(
        posts[0].body.data[0]["time/usage_start"],
        "2025-04-20T02:00:00Z"
    );
    assert_eq!(posts[0].body.month, "2025-04");
}

#[rstest]
#[case::missing_key(None)]
#[case::none(Some(CbfValue::None))]
#[case::empty_text(Some(CbfValue::Text(String::new())))]
fn missing_or_empty_usage_start_is_skipped_without_a_warning(#[case] start: Option<CbfValue>) {
    let mut skipped = text_row("2025-04-19T00:00:00Z");
    match start {
        Some(value) => skipped.insert("time/usage_start".to_owned(), value),
        None => skipped.remove("time/usage_start"),
    };
    let kept = text_row("2025-04-20T00:00:00Z");
    let (posts, logs) = capture_logs(|| posts_of(None, &[skipped, kept], None));
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].body.data.len(), 1);
    assert_eq!(logs, "");
}

#[test]
fn a_frame_without_any_usable_timestamp_posts_nothing() {
    let (posts, logs) = capture_logs(|| posts_of(None, &[text_row(""), text_row("garbage")], None));
    assert!(posts.is_empty());
    assert!(logs.contains("garbage"), "{logs}");
}

#[rstest]
#[case::garbage("not-a-date")]
#[case::bad_month("2025-13-45T00:00:00Z")]
#[case::truncated("2025-04")]
fn malformed_timestamps_are_skipped_with_a_warning_naming_the_timestamp(#[case] text: &str) {
    let data = [text_row(text), text_row("2025-04-19T00:00:00Z")];
    let (posts, logs) = capture_logs(|| posts_of(None, &data, None));
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].body.data.len(), 1);
    assert!(logs.contains("WARN"), "{logs}");
    assert!(
        logs.contains(&format!("Could not parse timestamp '{text}'")),
        "{logs}"
    );
}

#[rstest]
#[case::integer(CbfValue::Integer(20_250_419), "20250419")]
#[case::money(CbfValue::Money(Decimal::new(15, 1)), "1.5")]
fn a_non_text_usage_start_is_skipped_with_a_warning(#[case] start: CbfValue, #[case] shown: &str) {
    let (posts, logs) = capture_logs(|| posts_of(None, &[row(start, Decimal::ONE)], None));
    assert!(posts.is_empty());
    assert!(
        logs.contains(&format!("Could not process timestamp '{shown}'")),
        "{logs}"
    );
}

#[rstest]
#[case::tokyo_midnight(Some("Asia/Tokyo"), (2025, 4, 19, 0, 0, 0), "2025-04-18T15:00:00Z", "2025-04")]
#[case::tokyo_month_start(Some("Asia/Tokyo"), (2025, 5, 1, 0, 0, 0), "2025-04-30T15:00:00Z", "2025-04")]
#[case::tokyo_evening(Some("Asia/Tokyo"), (2025, 4, 19, 23, 59, 59), "2025-04-19T14:59:59Z", "2025-04")]
#[case::new_york_midnight(Some("America/New_York"), (2025, 7, 1, 0, 0, 0), "2025-07-01T04:00:00Z", "2025-07")]
#[case::utc(Some("UTC"), (2025, 4, 19, 0, 0, 0), "2025-04-19T00:00:00Z", "2025-04")]
#[case::unset(None, (2025, 4, 19, 0, 0, 0), "2025-04-19T00:00:00Z", "2025-04")]
fn a_datetime_usage_start_is_wall_clock_time_in_the_user_timezone(
    #[case] zone: Option<&str>,
    #[case] wall_clock: (i32, u32, u32, u32, u32, u32),
    #[case] expected: &str,
    #[case] month: &str,
) {
    let (year, month_number, day, hour, minute, second) = wall_clock;
    let moment = Utc
        .with_ymd_and_hms(year, month_number, day, hour, minute, second)
        .unwrap();
    let posts = posts_of(zone, &[row(CbfValue::DateTime(moment), Decimal::ONE)], None);
    assert_eq!(posts[0].body.data[0]["time/usage_start"], expected);
    assert_eq!(posts[0].body.month, month);
}

#[rstest]
#[case::whole("1.0", "1")]
#[case::trailing_zeros("1.1000000000", "1.1")]
#[case::integer_like("100", "100")]
#[case::zero("0.00", "0")]
#[case::small("0.0000000123", "0.0000000123")]
#[case::below_precision("0.00000000001", "0")]
#[case::rounds_to_ten_places("0.12345678916", "0.1234567892")]
#[case::negative("-2.50", "-2.5")]
#[case::large("123456789012345.678", "123456789012345.678")]
fn money_formats_with_ten_places_and_no_trailing_zeros(
    #[case] value: &str,
    #[case] expected: &str,
) {
    assert_eq!(format_decimal(value.parse().unwrap()), expected);
}

#[test]
fn numbers_become_strings_and_nulls_are_stripped() {
    let posts = posts_of(None, &[text_row("2025-04-19T00:00:00Z")], None);
    let record = &posts[0].body.data[0];
    assert_eq!(record["usage/amount"], "42");
    assert_eq!(record["cost/cost"], "1.5");
    assert!(!record.contains_key("resource/tag:team"));
}

#[rstest]
#[case::empty("")]
#[case::unparsable("garbage")]
fn an_empty_or_unparsable_usage_start_in_a_record_becomes_the_current_time(#[case] text: &str) {
    let converted = convert_record(&text_row(text), &TimeZone::UTC, &FixedClock);
    assert_eq!(converted["time/usage_start"], "2030-01-02T03:04:05Z");
}

#[test]
fn a_failed_post_stops_the_remaining_days() {
    let transport = FakeTransport {
        fail_on_call: Some(0),
        posted: Arc::default(),
    };
    let posted = Arc::clone(&transport.posted);
    let data = [
        text_row("2025-04-19T00:00:00Z"),
        text_row("2025-04-20T00:00:00Z"),
    ];
    let error = complete(streamer(None, transport).send_batched(&data, None)).unwrap_err();
    assert_eq!(posted.lock().unwrap().len(), 1);
    let DeliveryError::<Refused>::Send { date, .. } = &error;
    assert_eq!(date.to_string(), "2025-04-19");
    assert!(error.source().unwrap().is::<Refused>());
}
