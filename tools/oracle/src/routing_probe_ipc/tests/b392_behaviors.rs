use std::fs;

use rstest::rstest;

use super::super::child::run_bounded;
use super::super::{
    BUILD_DEADLINE, MAX_MESSAGE_BYTES, OPERATION_DEADLINE, ProbeFailure, ProbeReply, ProbeRequest,
    parse_message, probe,
};
use super::{REPLY, REQUEST, echo_reply, request, scratch, shell};

#[rstest]
fn request_carries_clock_samples_and_operation() {
    assert_eq!(serde_json::to_string(&request()).unwrap(), REQUEST);
    let parsed: ProbeRequest = parse_message(REQUEST.as_bytes()).unwrap();
    assert_eq!(parsed, request());
}

#[rstest]
#[case::ok_with_warnings(REPLY, echo_reply())]
#[case::samples_exhausted(
    r#"{"outcome":"error","kind":"samples_exhausted","message":"no sample left"}"#,
    ProbeReply::Error { kind: ProbeFailure::SamplesExhausted, message: "no sample left".to_owned() }
)]
#[case::unknown_operation(
    r#"{"outcome":"error","kind":"unknown_operation","message":"shuffle"}"#,
    ProbeReply::Error { kind: ProbeFailure::UnknownOperation, message: "shuffle".to_owned() }
)]
fn replies_keep_outcome_and_warnings(#[case] text: &str, #[case] expected: ProbeReply) {
    let reply: ProbeReply = parse_message(text.as_bytes()).unwrap();
    assert_eq!(reply, expected);
}

#[tokio::test]
async fn probe_sends_the_request_and_keeps_stderr_apart() {
    let received = scratch("received");
    let command = shell(
        r#"cat > "$1"; printf '%s' "$2"; printf 'Selected deployment\n' >&2"#,
        &[received.to_str().unwrap(), REPLY],
    );
    let answer = probe(&command, &request(), OPERATION_DEADLINE)
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(&received).unwrap(), REQUEST);
    assert_eq!(answer.reply, echo_reply());
    assert_eq!(answer.stderr.bytes, b"Selected deployment\n");
    assert!(!answer.stderr.truncated);
    fs::remove_file(received).unwrap();
}

#[tokio::test]
async fn oversized_stderr_is_truncated_and_the_reply_still_counts() {
    let command = shell(
        r#"cat > /dev/null; head -c 1048600 /dev/zero >&2; printf '%s' "$1""#,
        &[REPLY],
    );
    let answer = probe(&command, &request(), OPERATION_DEADLINE)
        .await
        .unwrap();
    assert_eq!(answer.reply, echo_reply());
    assert_eq!(answer.stderr.bytes.len(), MAX_MESSAGE_BYTES);
    assert!(answer.stderr.truncated);
}

#[tokio::test]
async fn bounded_run_reports_status_and_both_streams() {
    let command = shell("printf built; printf warned >&2; exit 3", &[]);
    let output = run_bounded(&command, b"", BUILD_DEADLINE).await.unwrap();
    assert_eq!(output.status.code(), Some(3_i32));
    assert_eq!(output.stdout.bytes, b"built");
    assert_eq!(output.stderr.bytes, b"warned");
    assert!(!output.stdout.truncated && !output.stderr.truncated);
}
