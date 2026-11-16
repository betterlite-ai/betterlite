#![cfg(test)]

use chrono::FixedOffset;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{Stamp, stamp};
use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::time::FixedLocalTimeResolver;

fn read(text: &str) -> Result<Stamp, LegacyMetricsError> {
    let kwargs: MetadataMap = serde_json::from_str(text).expect("kwargs");
    let resolver = FixedLocalTimeResolver {
        offset: FixedOffset::east_opt(3600).expect("offset"),
    };
    stamp(&kwargs, "moment", &resolver)
}

#[rstest]
#[case::absent("{}", Stamp::Absent)]
#[case::null(r#"{"moment": null}"#, Stamp::Absent)]
#[case::integer(r#"{"moment": 12}"#, Stamp::At(Decimal::from(12_i64)))]
#[case::float(r#"{"moment": 12.5}"#, Stamp::At(Decimal::new(125, 1)))]
#[case::text_with_microseconds(
    r#"{"moment": "1970-01-01 01:00:10.250000"}"#,
    Stamp::At(Decimal::new(1025, 2))
)]
#[case::text_without_microseconds(
    r#"{"moment": "1970-01-01 01:00:10"}"#,
    Stamp::At(Decimal::from(10_i64))
)]
#[case::text_that_is_not_a_time(r#"{"moment": "tomorrow"}"#, Stamp::Unparsed)]
#[case::iso_text_with_a_t(r#"{"moment": "1970-01-01T01:00:10"}"#, Stamp::Unparsed)]
fn a_timestamp_is_a_number_or_strptime_text(#[case] text: &str, #[case] expected: Stamp) {
    assert_eq!(read(text).expect("stamp"), expected);
}

#[rstest]
#[case::bool(r#"{"moment": true}"#)]
#[case::list(r#"{"moment": [1]}"#)]
#[case::object(r#"{"moment": {"a": 1}}"#)]
fn any_other_type_is_an_error_naming_the_field(#[case] text: &str) {
    let error = read(text).expect_err("wrong type");
    assert!(matches!(
        error,
        LegacyMetricsError::WrongType {
            field: "moment",
            ..
        }
    ));
}
