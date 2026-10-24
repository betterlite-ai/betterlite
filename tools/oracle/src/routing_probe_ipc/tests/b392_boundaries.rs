use std::fs;
use std::time::Duration;

use rstest::rstest;
use tokio::time;

use super::super::child::run_bounded;
use super::super::{
    BUILD_DEADLINE, MAX_MESSAGE_BYTES, OPERATION_DEADLINE, ProbeReply, ProbeRequest, ProbeSample,
    parse_message, probe,
};
use super::{REQUEST, assert_gone, read_pid, request, scratch, shell};
use crate::OracleError;

#[rstest]
fn limits_match_the_task() {
    assert_eq!(MAX_MESSAGE_BYTES, 1_usize << 20_u32);
    assert_eq!(OPERATION_DEADLINE, Duration::from_secs(30));
    assert_eq!(BUILD_DEADLINE, Duration::from_secs(300));
}

#[rstest]
#[case::at_limit(MAX_MESSAGE_BYTES, true)]
#[case::over_limit(MAX_MESSAGE_BYTES + 1, false)]
fn parser_stops_at_one_mebibyte(#[case] size: usize, #[case] accepted: bool) {
    let mut padded = REQUEST.as_bytes().to_vec();
    padded.resize(size, b' ');
    let parsed = parse_message::<ProbeRequest>(&padded);
    assert_eq!(parsed.is_ok(), accepted);
    assert!(accepted || matches!(parsed, Err(OracleError::ProbeTooLarge)));
}

#[rstest]
#[case::largest((1_u64 << 53_u32) - 1, true)]
#[case::limit(1_u64 << 53_u32, false)]
fn samples_stay_below_two_to_the_53(#[case] value: u64, #[case] accepted: bool) {
    let sample = ProbeSample::try_from(value);
    assert_eq!(sample.is_ok(), accepted);
    assert!(
        accepted || matches!(sample, Err(OracleError::ProbeSample(rejected)) if rejected == value)
    );
    let text = REQUEST.replace("9007199254740991", &value.to_string());
    assert_eq!(
        parse_message::<ProbeRequest>(text.as_bytes()).is_ok(),
        accepted
    );
}

#[rstest]
#[case::unknown_operation(
    r#"{"clock":{"unix_millis":1,"monotonic_millis":1},"samples":[],"operation":{"shuffle":{}}}"#
)]
#[case::two_operations(r#"{"clock":{"unix_millis":1,"monotonic_millis":1},"samples":[],"operation":{"echo":{"text":"","draws":0,"warnings":[]},"shuffle":{}}}"#)]
#[case::extra_request_field(r#"{"clock":{"unix_millis":1,"monotonic_millis":1},"samples":[],"seed":4,"operation":{"echo":{"text":"","draws":0,"warnings":[]}}}"#)]
#[case::extra_input_field(r#"{"clock":{"unix_millis":1,"monotonic_millis":1},"samples":[],"operation":{"echo":{"text":"","draws":0,"warnings":[],"model":"x"}}}"#)]
#[case::negative_clock(r#"{"clock":{"unix_millis":-1,"monotonic_millis":1},"samples":[],"operation":{"echo":{"text":"","draws":0,"warnings":[]}}}"#)]
#[case::float_sample(r#"{"clock":{"unix_millis":1,"monotonic_millis":1},"samples":[0.5],"operation":{"echo":{"text":"","draws":0,"warnings":[]}}}"#)]
#[case::missing_samples(r#"{"clock":{"unix_millis":1,"monotonic_millis":1},"operation":{"echo":{"text":"","draws":0,"warnings":[]}}}"#)]
#[case::trailing_bytes(r#"{"clock":{"unix_millis":1,"monotonic_millis":1},"samples":[],"operation":{"echo":{"text":"","draws":0,"warnings":[]}}} {}"#)]
fn requests_outside_the_schema_are_malformed(#[case] text: &str) {
    let parsed = parse_message::<ProbeRequest>(text.as_bytes());
    assert!(matches!(parsed, Err(OracleError::ProbeMalformed(_))));
}

#[rstest]
#[case::empty("")]
#[case::unknown_outcome(r#"{"outcome":"skipped","warnings":[]}"#)]
#[case::unknown_kind(r#"{"outcome":"error","kind":"timeout","message":""}"#)]
#[case::extra_field(r#"{"outcome":"error","kind":"invalid_request","message":"","trace":""}"#)]
#[case::missing_warnings(r#"{"outcome":"ok","output":{"echo":{"text":"","clock":{"unix_millis":1,"monotonic_millis":1},"samples":[]}}}"#)]
#[case::unknown_output(r#"{"outcome":"ok","output":{"shuffle":{}},"warnings":[]}"#)]
fn replies_outside_the_schema_are_malformed(#[case] text: &str) {
    let parsed = parse_message::<ProbeReply>(text.as_bytes());
    assert!(matches!(parsed, Err(OracleError::ProbeMalformed(_))));
}

#[rstest]
#[case::oversized_reply(
    "cat > /dev/null; head -c 1048577 /dev/zero",
    |error: &OracleError| matches!(error, OracleError::ProbeTooLarge)
)]
#[case::garbage_reply(
    "cat > /dev/null; printf 'not json'",
    |error: &OracleError| matches!(error, OracleError::ProbeMalformed(_))
)]
#[case::crashed(
    "cat > /dev/null; printf 'Traceback\n' >&2; exit 1",
    |error: &OracleError| matches!(error, OracleError::ProbeExited { status, stderr } if status.code() == Some(1_i32) && stderr == b"Traceback\n")
)]
#[tokio::test]
async fn child_failures_are_typed(
    #[case] script: &str,
    #[case] expected: fn(&OracleError) -> bool,
) {
    let error = probe(&shell(script, &[]), &request(), OPERATION_DEADLINE)
        .await
        .unwrap_err();
    assert!(expected(&error), "{error:?}");
}

#[tokio::test]
async fn oversized_request_never_starts_the_child() {
    let marker = scratch("started");
    let command = shell(r#"touch "$1""#, &[marker.to_str().unwrap()]);
    let input = vec![b' '; MAX_MESSAGE_BYTES + 1];
    let outcome = run_bounded(&command, &input, OPERATION_DEADLINE).await;
    assert!(matches!(outcome, Err(OracleError::ProbeTooLarge)));
    assert!(!marker.exists());
}

#[tokio::test]
async fn deadline_kills_the_child() {
    let pid_path = scratch("deadline");
    let command = shell(
        r#"echo $$ > "$1"; exec sleep 30"#,
        &[pid_path.to_str().unwrap()],
    );
    let deadline = Duration::from_millis(300);
    let outcome = probe(&command, &request(), deadline).await;
    assert!(matches!(outcome, Err(OracleError::ProbeTimeout(elapsed)) if elapsed == deadline));
    assert_gone(&read_pid(&pid_path).await).await;
    fs::remove_file(pid_path).unwrap();
}

#[tokio::test]
async fn dropping_the_probe_kills_the_child() {
    let pid_path = scratch("cancelled");
    let command = shell(
        r#"echo $$ > "$1"; exec sleep 30"#,
        &[pid_path.to_str().unwrap()],
    );
    let echo = request();
    let probing = probe(&command, &echo, OPERATION_DEADLINE);
    let cancelled = time::timeout(Duration::from_secs(2), probing).await;
    cancelled.unwrap_err();
    assert_gone(&read_pid(&pid_path).await).await;
    fs::remove_file(pid_path).unwrap();
}
