use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::from_utf8;
use std::time::Duration;

use crate::OracleError;
use crate::process::sync_litellm;
use crate::router_probes::{ProbeOperation, Reply, Request, UnbuiltInput, execute};
use crate::routing_component_oracle::python_command;
use crate::routing_probe_ipc::child::{ProbeCommand, run_bounded};
use crate::routing_probe_ipc::{
    MAX_MESSAGE_BYTES, OPERATION_DEADLINE, ProbeClock, ProbeFailure, parse_message,
};

const CLOCK: &str = r#""clock":{"unix_millis":1700000000123,"monotonic_millis":42000}"#;
const SINGLE_CANDIDATE: &str = r#"{"single_candidate":{"deployments":[{"id":"only"}]}}"#;

fn request_text(samples: &str, operation: &str) -> String {
    format!("{{{CLOCK},\"samples\":{samples},\"operation\":{operation}}}")
}

#[test]
fn a_sample_at_or_above_the_draw_limit_is_rejected() {
    let text = request_text("[9007199254740992]", SINGLE_CANDIDATE);
    parse_message::<Request>(text.as_bytes()).unwrap_err();
}

#[test]
fn a_message_past_the_limit_is_rejected_before_parsing() {
    let oversized = vec![b'{'; MAX_MESSAGE_BYTES.saturating_add(1)];
    assert!(matches!(
        parse_message::<Request>(&oversized),
        Err(OracleError::ProbeTooLarge)
    ));
}

#[test]
fn an_operation_the_registry_never_declared_is_malformed() {
    let text = request_text("[0]", r#"{"no_such_operation":{}}"#);
    assert!(matches!(
        parse_message::<Request>(text.as_bytes()),
        Err(OracleError::ProbeMalformed(_))
    ));
}

#[test]
fn an_operation_carries_exactly_one_payload() {
    let text = request_text("[0]", r#"{"access_windows":{},"single_candidate":{}}"#);
    parse_message::<Request>(text.as_bytes()).unwrap_err();
}

#[test]
fn a_stub_rejects_payload_fields_it_does_not_model() {
    let text = request_text("[0]", r#"{"access_windows":{"extra":1}}"#);
    parse_message::<Request>(text.as_bytes()).unwrap_err();
}

#[test]
fn a_reply_with_an_undeclared_failure_kind_is_malformed() {
    let text = r#"{"outcome":"error","kind":"not_a_failure","message":"x"}"#;
    assert!(matches!(
        parse_message::<Reply>(text.as_bytes()),
        Err(OracleError::ProbeMalformed(_))
    ));
}

#[tokio::test]
async fn a_probe_that_never_answers_is_stopped_at_the_deadline() {
    let deadline = Duration::from_millis(100);
    let command = ProbeCommand {
        program: PathBuf::from("sh"),
        args: vec![OsString::from("-c"), OsString::from("sleep 5")],
        cwd: env::temp_dir(),
    };
    let error = run_bounded(&command, b"", deadline).await.unwrap_err();
    assert!(matches!(error, OracleError::ProbeTimeout(waited) if waited == deadline));
}

#[tokio::test]
async fn a_stub_answers_not_built_on_both_sides() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    sync_litellm(&root).unwrap();
    let request = Request {
        clock: ProbeClock {
            unix_millis: 1_700_000_000_123,
            monotonic_millis: 42_000,
        },
        samples: Vec::new(),
        operation: ProbeOperation::AccessWindows(UnbuiltInput::default()),
    };
    let bytes = serde_json::to_vec(&request).unwrap();
    let command = python_command(&root);
    let output = run_bounded(&command, &bytes, OPERATION_DEADLINE)
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{:?}",
        from_utf8(&output.stderr.bytes)
    );
    let python: Reply = parse_message(&output.stdout.bytes).unwrap();
    let expected = Reply::Error {
        kind: ProbeFailure::UnknownOperation,
        message: "access_windows is not built".to_owned(),
    };
    assert_eq!(python, expected);
    assert_eq!(execute(&request), expected);
}
