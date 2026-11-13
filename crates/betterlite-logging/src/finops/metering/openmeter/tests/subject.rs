#![cfg(test)]

use rstest::rstest;

use super::support::settings_with;
use crate::finops::metering::openmeter::payload::{OpenMeterPayloadError, build};
use crate::finops::metering::support::{FixedClock, event};

#[test]
fn subject_prefers_the_request_user_by_default() {
    let payload = build(&settings_with(&[]), &FixedClock, &event()).unwrap();
    assert_eq!(payload.subject, "end-user-1");
}

#[test]
fn subject_falls_back_to_the_key_bound_user() {
    let mut call = event();
    call.request_user = None;
    let payload = build(&settings_with(&[]), &FixedClock, &call).unwrap();
    assert_eq!(payload.subject, "key-user-1");
}

#[rstest]
#[case::lowercase("false", "key-user-1")]
#[case::uppercase("FALSE", "key-user-1")]
#[case::mixed_case("False", "key-user-1")]
#[case::no("no", "key-user-1")]
#[case::zero("0", "key-user-1")]
#[case::off("off", "key-user-1")]
#[case::true_value("true", "end-user-1")]
#[case::true_uppercase("TRUE", "end-user-1")]
#[case::yes("yes", "end-user-1")]
#[case::one("1", "end-user-1")]
#[case::on("on", "end-user-1")]
fn subject_follows_the_trust_flag(#[case] value: &'static str, #[case] expected: &'static str) {
    let payload = build(
        &settings_with(&[("OPENMETER_TRUST_REQUEST_USER", value)]),
        &FixedClock,
        &event(),
    )
    .unwrap();
    assert_eq!(payload.subject, expected);
}

#[test]
fn subject_requires_a_user_when_both_sources_are_missing() {
    let mut call = event();
    call.request_user = None;
    call.user_api_key_user_id = None;
    let error = build(&settings_with(&[]), &FixedClock, &call).unwrap_err();
    assert!(matches!(error, OpenMeterPayloadError::MissingSubject));
}

#[test]
fn subject_requires_a_key_bound_user_when_trust_is_disabled() {
    let mut call = event();
    call.user_api_key_user_id = None;
    let error = build(
        &settings_with(&[("OPENMETER_TRUST_REQUEST_USER", "false")]),
        &FixedClock,
        &call,
    )
    .unwrap_err();
    assert!(matches!(error, OpenMeterPayloadError::MissingSubject));
}
