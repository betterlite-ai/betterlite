#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{STREAM_DURATION_ENV, StreamDurationError};

#[rstest]
#[case::unset(None, Ok(None))]
#[case::empty(Some(""), Ok(None))]
#[case::blank(Some("   "), Ok(None))]
#[case::plain(Some("2.5"), Ok(Some(2.5_f64)))]
#[case::padded(Some(" +2.5 "), Ok(Some(2.5_f64)))]
#[case::zero(Some("0"), Ok(Some(0.0_f64)))]
#[case::negative(Some("-1"), Ok(Some(-1.0_f64)))]
#[case::exponent(Some("1e20"), Ok(Some(1e20_f64)))]
#[case::tiny(Some("1e-6"), Ok(Some(1e-6_f64)))]
#[case::nan(Some("nan"), Ok(Some(f64::NAN)))]
#[case::upper_nan(Some("NaN"), Ok(Some(f64::NAN)))]
#[case::infinity(Some("Infinity"), Ok(Some(f64::INFINITY)))]
#[case::negative_infinity(Some("-inf"), Ok(Some(f64::NEG_INFINITY)))]
#[case::word(Some("bad"), Err(StreamDurationError::Invalid))]
#[case::suffix(Some("2.5s"), Err(StreamDurationError::Invalid))]
#[case::underscore(Some("1_000"), Err(StreamDurationError::Invalid))]
fn loads_stream_duration_settings(
    #[case] raw: Option<&str>,
    #[case] expected: Result<Option<f64>, StreamDurationError>,
) {
    let mut environment = BTreeMap::new();
    if let Some(value) = raw {
        environment.insert(STREAM_DURATION_ENV.to_owned(), value.to_owned());
    }
    let loaded = super::loads_stream_duration_settings(&environment)
        .map(|settings| settings.max_streaming_duration_seconds);
    match (loaded, expected) {
        (Ok(Some(actual)), Ok(Some(wanted))) if wanted.is_nan() => {
            assert!(actual.is_nan(), "{raw:?}");
        }
        (actual, wanted) => assert_eq!(actual, wanted, "{raw:?}"),
    }
}
