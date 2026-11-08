use rstest::rstest;

use crate::fields::timestamps::{Moment, get_response_ms};

use super::{moment, seconds};

#[rstest]
#[case::floats(
    Some(Moment::Float(seconds("1704067200.5"))),
    Some(Moment::Float(seconds("1704067202.0"))),
    "1500.0"
)]
#[case::integers(
    Some(Moment::Integer(1_704_067_200)),
    Some(Moment::Integer(1_704_067_202)),
    "2000.0"
)]
#[case::datetimes(
    Some(moment("2024-01-01T00:00:00+00:00")),
    Some(moment("2024-01-01T00:00:00.250+00:00")),
    "250.0"
)]
fn logging_fields_behavior_tests_behavior(
    #[case] start: Option<Moment>,
    #[case] end: Option<Moment>,
    #[case] expected: &str,
) {
    let now = Moment::Float(seconds("1800000000.0"));
    assert_eq!(
        get_response_ms(start.as_ref(), end.as_ref(), &now),
        Ok(seconds(expected))
    );
}

#[test]
fn a_missing_timestamp_falls_back_to_the_injected_now() {
    let now = Moment::Float(seconds("1704067200.5"));
    let end = Moment::Float(seconds("1704067202.0"));
    assert_eq!(
        get_response_ms(None, Some(&end), &now),
        Ok(seconds("1500.0"))
    );
    assert_eq!(
        get_response_ms(Some(&end), None, &now),
        Ok(seconds("-1500.0"))
    );
}
