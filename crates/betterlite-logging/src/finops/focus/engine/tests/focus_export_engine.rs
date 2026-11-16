use std::str::FromStr as _;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::finops::focus::UsageData;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::finops::focus::destinations::factory::error::FactoryError;
use crate::finops::focus::engine::filename;
use crate::finops::focus::engine::summary::{SummaryError, count_unique, sum_column};
use crate::finops::focus::engine::{ExportError, ExportSettings, FocusExportEngine, format_for};
use crate::focus_records::destinations::FocusTimeWindow;
use crate::focus_records::serializers::FocusFormat;
use crate::focus_records::table::FocusTable;
use crate::focus_records::transformer::transform;

use super::support::{
    FailingFactory, FixedClock, PREFIX, RecordingFactory, RecordingSource, data_row, harness,
    harness_with_transform, number, overrides_with_bucket_name, refusing_transform, settings, text,
    timestamp, usage_row, window,
};

#[test]
fn init_builds_the_destination_with_the_settings() {
    let factory = RecordingFactory::default();
    let configuration = ExportSettings {
        provider: String::from("s3"),
        export_format: String::from("csv"),
        prefix: String::from(PREFIX),
        destination_config: overrides_with_bucket_name("focus-bucket"),
    };
    let clock = FixedClock {
        now: timestamp("2026-03-04T05:06:07Z"),
    };
    let engine = FocusExportEngine::new(
        configuration,
        factory.clone(),
        RecordingSource::default(),
        clock,
    )
    .expect("the recording factory builds a destination");
    let calls = factory.calls();
    assert_eq!(calls.len(), 1);
    let call = calls.first().expect("the factory was called once");
    assert_eq!(call.provider, "s3");
    assert_eq!(call.prefix, PREFIX);
    assert_eq!(call.overrides.bucket_name.as_deref(), Some("focus-bucket"));
    assert_eq!(engine.format, FocusFormat::Csv);
}

#[test]
fn init_selects_the_parquet_serializer() {
    let engine = FocusExportEngine::new(
        settings("parquet"),
        RecordingFactory::default(),
        RecordingSource::default(),
        FixedClock {
            now: timestamp("2026-03-04T05:06:07Z"),
        },
    )
    .expect("the recording factory builds a destination");
    assert_eq!(engine.format, FocusFormat::Parquet);
}

#[test]
fn init_creates_the_destination_before_checking_the_format() {
    let result = FocusExportEngine::new(
        settings("json"),
        FailingFactory,
        RecordingSource::default(),
        FixedClock {
            now: timestamp("2026-03-04T05:06:07Z"),
        },
    );
    let error = result.err().expect("the factory refuses the destination");
    assert!(matches!(
        &error,
        ExportError::Destination(FactoryError::MissingS3BucketName)
    ));
}

#[test]
fn serializer_uses_csv_for_the_csv_format() {
    assert_eq!(format_for("csv"), Some(FocusFormat::Csv));
}

#[test]
fn serializer_uses_parquet_for_the_parquet_format() {
    assert_eq!(format_for("parquet"), Some(FocusFormat::Parquet));
}

#[test]
fn serializer_rejects_an_unsupported_format() {
    let result = FocusExportEngine::new(
        settings("json"),
        RecordingFactory::default(),
        RecordingSource::default(),
        FixedClock {
            now: timestamp("2026-03-04T05:06:07Z"),
        },
    );
    let error = result.err().expect("the format is not supported");
    assert_eq!(
        error.to_string(),
        "Export format 'json' not supported. Use 'parquet' or 'csv'."
    );
}

#[test]
fn serializer_rejects_an_upper_case_format() {
    let result = FocusExportEngine::new(
        settings("CSV"),
        RecordingFactory::default(),
        RecordingSource::default(),
        FixedClock {
            now: timestamp("2026-03-04T05:06:07Z"),
        },
    );
    let error = result.err().expect("the format is matched exactly");
    assert!(matches!(&error, ExportError::UnsupportedFormat { .. }));
}

#[tokio::test]
async fn export_all_skips_when_the_data_is_empty() {
    let harness = harness_with_transform(
        "csv",
        Vec::new(),
        timestamp("2026-03-04T05:06:07Z"),
        refusing_transform,
    );
    harness
        .engine
        .export_all(Some(10))
        .await
        .expect("an empty export succeeds");
    assert!(harness.deliveries().is_empty());
    let calls = harness.source_calls();
    assert_eq!(calls.len(), 1);
    let call = calls.first().expect("the source was read once");
    assert_eq!(call.limit, Some(10));
    assert_eq!(call.start_time_utc, None);
    assert_eq!(call.end_time_utc, None);
}

#[tokio::test]
async fn export_all_skips_when_the_normalized_data_is_empty() {
    let harness = harness_with_transform(
        "csv",
        vec![usage_row()],
        timestamp("2026-03-04T05:06:07Z"),
        |_rows| {
            Ok(FocusTable {
                rows: 0,
                columns: Vec::new(),
            })
        },
    );
    harness
        .engine
        .export_all(None)
        .await
        .expect("an empty normalized frame succeeds");
    assert!(harness.deliveries().is_empty());
}

#[tokio::test]
async fn export_all_passes_the_limit_without_a_window() {
    let harness = harness("csv", vec![usage_row()], timestamp("2026-03-04T05:06:07Z"));
    harness
        .engine
        .export_all(Some(3))
        .await
        .expect("the export succeeds");
    let calls = harness.source_calls();
    let call = calls.first().expect("the source was read once");
    assert_eq!(call.limit, Some(3));
    assert_eq!(call.start_time_utc, None);
    assert_eq!(call.end_time_utc, None);
    assert_eq!(harness.deliveries().len(), 1);
}

#[tokio::test]
async fn export_all_delivers_csv_of_the_normalized_rows() {
    let rows = vec![usage_row()];
    let table = transform(&rows).expect("the usage row normalizes");
    let expected = FocusFormat::Csv
        .encode(&table)
        .expect("the table serializes");
    let harness = harness("csv", rows, timestamp("2026-03-04T05:06:07Z"));
    harness
        .engine
        .export_all(None)
        .await
        .expect("the export succeeds");
    let deliveries = harness.deliveries();
    let delivery = deliveries.first().expect("the export is delivered");
    assert_eq!(delivery.content, expected);
    assert_eq!(
        delivery.filename,
        "usage_20260304T000000Z_20260304T050607Z.csv"
    );
}

#[tokio::test]
async fn export_all_window_spans_today_midnight_to_now() {
    let now = timestamp("2026-03-04T05:06:07Z");
    let harness = harness("csv", vec![usage_row()], now);
    harness
        .engine
        .export_all(None)
        .await
        .expect("the export succeeds");
    let deliveries = harness.deliveries();
    let delivery = deliveries.first().expect("the export is delivered");
    assert_eq!(
        delivery.window.start_time,
        timestamp("2026-03-04T00:00:00Z")
    );
    assert_eq!(delivery.window.end_time, now);
    assert_eq!(delivery.window.frequency, "all");
    assert_eq!(
        delivery.filename,
        "usage_20260304T000000Z_20260304T050607Z.csv"
    );
}

#[tokio::test]
async fn export_all_window_starts_and_ends_together_at_midnight() {
    let now = timestamp("2026-03-04T00:00:00Z");
    let harness = harness("csv", vec![usage_row()], now);
    harness
        .engine
        .export_all(None)
        .await
        .expect("the export succeeds");
    let deliveries = harness.deliveries();
    let delivery = deliveries.first().expect("the export is delivered");
    assert_eq!(delivery.window.start_time, now);
    assert_eq!(delivery.window.end_time, now);
    assert_eq!(
        delivery.filename,
        "usage_20260304T000000Z_20260304T000000Z.csv"
    );
}

#[tokio::test]
async fn export_window_bounds_binds_both_ends_for_the_query() {
    let harness = harness("csv", vec![usage_row()], timestamp("2026-03-04T05:06:07Z"));
    let time_window = window();
    harness
        .engine
        .export_window(&time_window, Some(7))
        .await
        .expect("the export succeeds");
    let calls = harness.source_calls();
    let call = calls.first().expect("the source was read once");
    assert_eq!(call.limit, Some(7));
    assert_eq!(call.start_time_utc, Some(time_window.start_time));
    assert_eq!(call.end_time_utc, Some(time_window.end_time));
}

#[tokio::test]
async fn export_window_bounds_delivers_with_the_given_window() {
    let harness = harness("csv", vec![usage_row()], timestamp("2026-03-04T05:06:07Z"));
    let time_window = window();
    harness
        .engine
        .export_window(&time_window, None)
        .await
        .expect("the export succeeds");
    let deliveries = harness.deliveries();
    let delivery = deliveries.first().expect("the export is delivered");
    assert_eq!(delivery.window, time_window);
    assert_eq!(
        delivery.filename,
        "usage_20260102T030405Z_20260102T040506Z.csv"
    );
}

#[tokio::test]
async fn export_window_empty_skips_when_the_data_is_empty() {
    let harness = harness_with_transform(
        "csv",
        Vec::new(),
        timestamp("2026-03-04T05:06:07Z"),
        refusing_transform,
    );
    harness
        .engine
        .export_window(&window(), None)
        .await
        .expect("an empty export succeeds");
    assert!(harness.deliveries().is_empty());
}

#[tokio::test]
async fn export_window_empty_skips_when_the_normalized_data_is_empty() {
    let harness = harness_with_transform(
        "csv",
        vec![usage_row()],
        timestamp("2026-03-04T05:06:07Z"),
        |_rows| {
            Ok(FocusTable {
                rows: 0,
                columns: Vec::new(),
            })
        },
    );
    harness
        .engine
        .export_window(&window(), None)
        .await
        .expect("an empty normalized frame succeeds");
    assert!(harness.deliveries().is_empty());
}

#[tokio::test]
async fn serialize_and_upload_skips_an_empty_payload() {
    let harness = harness("csv", vec![usage_row()], timestamp("2026-03-04T05:06:07Z"));
    harness
        .engine
        .upload_payload(&[], &window())
        .await
        .expect("an empty payload is not delivered");
    assert!(harness.deliveries().is_empty());
}

#[tokio::test]
async fn serialize_and_upload_delivers_the_serialized_payload() {
    let rows = vec![usage_row()];
    let table = transform(&rows).expect("the usage row normalizes");
    let expected = FocusFormat::Csv
        .encode(&table)
        .expect("the table serializes");
    let harness = harness_with_transform("csv", rows, timestamp("2026-03-04T05:06:07Z"), transform);
    harness
        .engine
        .export_window(&window(), None)
        .await
        .expect("the payload is delivered");
    let deliveries = harness.deliveries();
    let delivery = deliveries.first().expect("the export is delivered");
    assert_eq!(delivery.content, expected);
    assert_eq!(delivery.window, window());
    assert_eq!(
        delivery.filename,
        "usage_20260102T030405Z_20260102T040506Z.csv"
    );
}

#[tokio::test]
async fn serialize_and_upload_uses_the_parquet_extension() {
    let rows = vec![usage_row()];
    let harness = harness_with_transform(
        "parquet",
        rows,
        timestamp("2026-03-04T05:06:07Z"),
        transform,
    );
    harness
        .engine
        .export_window(&window(), None)
        .await
        .expect("the payload is delivered");
    let deliveries = harness.deliveries();
    let delivery = deliveries.first().expect("the export is delivered");
    assert!(delivery.content.starts_with(b"PAR1"));
    assert_eq!(
        delivery.filename,
        "usage_20260102T030405Z_20260102T040506Z.parquet"
    );
}

#[test]
fn filename_format_uses_both_utc_timestamps() {
    let name = filename::build(&window(), "csv").expect("the extension is present");
    assert_eq!(name, "usage_20260102T030405Z_20260102T040506Z.csv");
}

#[test]
fn filename_format_carries_the_parquet_extension() {
    let name = filename::build(&window(), "parquet").expect("the extension is present");
    assert_eq!(name, "usage_20260102T030405Z_20260102T040506Z.parquet");
}

#[test]
fn filename_format_pads_every_field() {
    let time_window = FocusTimeWindow {
        start_time: timestamp("2026-09-08T01:02:03Z"),
        end_time: timestamp("2026-12-31T23:59:59Z"),
        frequency: String::from("daily"),
    };
    let name = filename::build(&time_window, "csv").expect("the extension is present");
    assert_eq!(name, "usage_20260908T010203Z_20261231T235959Z.csv");
}

#[test]
fn filename_error_when_the_extension_is_empty() {
    let error = filename::build(&window(), "").expect_err("the empty extension is refused");
    assert_eq!(
        error.to_string(),
        "Serializer must declare a file extension"
    );
}

#[test]
fn sum_column_returns_zero_for_an_empty_frame() {
    let rows: UsageData = Vec::new();
    assert_eq!(
        sum_column(&rows, "spend").expect("an empty frame sums to zero"),
        Decimal::ZERO
    );
}

#[test]
fn sum_column_returns_zero_for_a_missing_column() {
    let rows = vec![data_row(&[("spend", number("1.5"))])];
    assert_eq!(
        sum_column(&rows, "total_tokens").expect("a missing column sums to zero"),
        Decimal::ZERO
    );
}

#[test]
fn sum_column_treats_null_values_as_zero() {
    let rows = vec![
        data_row(&[("spend", JsonData::Null)]),
        data_row(&[("spend", number("1.5"))]),
    ];
    assert_eq!(
        sum_column(&rows, "spend").expect("nulls sum to zero"),
        Decimal::new(15, 1)
    );
}

#[rstest]
#[case::whole(vec!["1", "2"], "3")]
#[case::negative(vec!["-1.5", "2.5"], "1")]
#[case::exact_scale(vec!["0.1", "0.2"], "0.3")]
#[case::single(vec!["12.345678"], "12.345678")]
fn sum_column_adds_the_present_values(#[case] values: Vec<&str>, #[case] expected: &str) {
    let rows: UsageData = values
        .iter()
        .map(|value| data_row(&[("spend", number(value))]))
        .collect();
    let total = sum_column(&rows, "spend").expect("the values are numeric");
    assert_eq!(
        total,
        Decimal::from_str(expected).expect("the expected total is a decimal")
    );
}

#[test]
fn sum_column_rejects_a_text_value() {
    let rows = vec![data_row(&[("spend", text("1.5"))])];
    let error = sum_column(&rows, "spend").expect_err("text is not a number");
    assert!(matches!(error, SummaryError::NotNumeric { .. }));
}

#[test]
fn sum_column_reports_an_overflow() {
    let maximum = Decimal::MAX.to_string();
    let rows = vec![
        data_row(&[("spend", number(&maximum))]),
        data_row(&[("spend", number(&maximum))]),
    ];
    let error = sum_column(&rows, "spend").expect_err("the sum does not fit a decimal");
    assert!(matches!(error, SummaryError::Overflow { .. }));
}

#[test]
fn sum_column_reports_a_number_that_is_not_a_decimal() {
    let huge = "9999999999999999999999999999999999999999";
    let rows = vec![data_row(&[("spend", number(huge))])];
    let error = sum_column(&rows, "spend").expect_err("the number does not fit a decimal");
    assert!(matches!(error, SummaryError::NotDecimal { .. }));
}

#[test]
fn count_unique_returns_zero_for_an_empty_frame() {
    let rows: UsageData = Vec::new();
    assert_eq!(count_unique(&rows, "team_id"), 0);
}

#[test]
fn count_unique_returns_zero_for_a_missing_column() {
    let rows = vec![data_row(&[("model", text("gpt-4o"))])];
    assert_eq!(count_unique(&rows, "team_id"), 0);
}

#[test]
fn count_unique_counts_distinct_text_values() {
    let rows = vec![
        data_row(&[("team_id", text("team-1"))]),
        data_row(&[("team_id", text("team-2"))]),
        data_row(&[("team_id", text("team-1"))]),
    ];
    assert_eq!(count_unique(&rows, "team_id"), 2);
}

#[test]
fn count_unique_counts_distinct_numbers() {
    let rows = vec![
        data_row(&[("user_id", number("1"))]),
        data_row(&[("user_id", number("1"))]),
        data_row(&[("user_id", number("2"))]),
    ];
    assert_eq!(count_unique(&rows, "user_id"), 2);
}

#[test]
fn count_unique_counts_null_as_one_value() {
    let rows = vec![
        data_row(&[("team_id", JsonData::Null)]),
        data_row(&[("team_id", JsonData::Null)]),
        data_row(&[("team_id", text("team-1"))]),
    ];
    assert_eq!(count_unique(&rows, "team_id"), 2);
}

#[test]
fn count_unique_skips_rows_without_the_key() {
    let rows = vec![
        data_row(&[("team_id", text("team-1"))]),
        data_row(&[("model", text("gpt-4o"))]),
        data_row(&[("team_id", text("team-1"))]),
    ];
    assert_eq!(count_unique(&rows, "team_id"), 1);
}
