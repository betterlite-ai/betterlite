use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::from_utf8;
use std::time::Duration;

use rand::TryRng as _;
use rstest::rstest;

use crate::OracleError;
use crate::case::Ignore;
use crate::cli::Options;
use crate::process::sync_litellm;
use crate::router_probes::random::InjectedRandom;
use crate::router_probes::single_candidate::{Deployment, Input, Output};
use crate::router_probes::{
    ProbeOperation, ProbeOutput, Reply, Request, Sample, UnbuiltInput, execute,
};
use crate::routing_component_oracle::{
    Commands, ProbeCase, ProbeCompare, ProbeStep, compare_replies, python_command, run_case,
    validate_probe_case,
};
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

fn step() -> ProbeStep {
    ProbeStep {
        clock: ProbeClock {
            unix_millis: 1,
            monotonic_millis: 2,
        },
        samples: vec![Sample::try_from(0).unwrap()],
        operation: ProbeOperation::SingleCandidate(Input {
            deployments: vec![Deployment {
                id: "only".to_owned(),
                weight: 3,
                rpm: 0,
                tpm: 0,
            }],
        }),
    }
}

fn probe_case(kind: &str, name: &str) -> ProbeCase {
    ProbeCase {
        kind: kind.to_owned(),
        name: name.to_owned(),
        probes: vec![step()],
        compare: ProbeCompare::default(),
    }
}

fn shell(script: &str, arguments: &[&str]) -> ProbeCommand {
    let mut complete = vec![
        OsString::from("-c"),
        OsString::from(script),
        OsString::from("probe"),
    ];
    complete.extend(arguments.iter().map(OsString::from));
    ProbeCommand {
        program: PathBuf::from("sh"),
        args: complete,
        cwd: env::temp_dir(),
    }
}

#[test]
fn a_different_selection_is_reported_at_the_selected_field() {
    let first = Reply::Ok {
        output: ProbeOutput::SingleCandidate(Output {
            selected: "first".to_owned(),
        }),
        warnings: Vec::new(),
    };
    let second = Reply::Ok {
        output: ProbeOutput::SingleCandidate(Output {
            selected: "second".to_owned(),
        }),
        warnings: Vec::new(),
    };
    let difference = compare_replies(&first, &second, &[]).unwrap().unwrap();
    assert_eq!(difference.pointer, "/output/single_candidate/selected");
}

#[test]
fn an_ignored_message_removes_only_that_difference() {
    let first = Reply::Error {
        kind: ProbeFailure::InvalidRequest,
        message: "first wording".to_owned(),
    };
    let second = Reply::Error {
        kind: ProbeFailure::InvalidRequest,
        message: "second wording".to_owned(),
    };
    let difference = compare_replies(&first, &second, &[]).unwrap().unwrap();
    assert_eq!(difference.pointer, "/message");
    let ignores = [Ignore {
        pointer: "/message".to_owned(),
        why: "betterlite writes its own message".to_owned(),
    }];
    assert!(
        compare_replies(&first, &second, &ignores)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_probe_case_needs_the_probe_kind_and_a_name() {
    let path = Path::new("probe.yaml");
    assert!(matches!(
        validate_probe_case(path, &probe_case("helper", "case")),
        Err(OracleError::CaseInvalid { .. })
    ));
    assert!(matches!(
        validate_probe_case(path, &probe_case("probe", "")),
        Err(OracleError::CaseInvalid { .. })
    ));
    validate_probe_case(path, &probe_case("probe", "case")).unwrap();
}

#[test]
fn a_probe_case_needs_at_least_one_probe() {
    let mut case = probe_case("probe", "case");
    case.probes.clear();
    assert!(matches!(
        validate_probe_case(Path::new("probe.yaml"), &case),
        Err(OracleError::CaseInvalid { .. })
    ));
}

const MATCHING_REPLY: &str =
    r#"{"outcome":"ok","output":{"single_candidate":{"selected":"probe"}},"warnings":[]}"#;
const DIFFERENT_REPLY: &str =
    r#"{"outcome":"ok","output":{"single_candidate":{"selected":"native"}},"warnings":[]}"#;

#[tokio::test]
async fn a_difference_in_one_step_fails_the_whole_case() {
    let directory = tempfile::tempdir().unwrap();
    let counter = directory.path().join("draws");
    let counter_path = counter.to_str().unwrap();
    let native_script = r#"cat > /dev/null
count=$(cat "$1" 2>/dev/null || echo 0)
echo $((count + 1)) > "$1"
if [ "$count" = "0" ]; then printf '%s' "$2"; else printf '%s' "$3"; fi"#;
    let python_script = r#"cat > /dev/null
printf '%s' "$2""#;
    let commands = Commands {
        native: shell(
            native_script,
            &[counter_path, DIFFERENT_REPLY, MATCHING_REPLY],
        ),
        python: shell(python_script, &[counter_path, MATCHING_REPLY]),
    };
    let case = ProbeCase {
        kind: "probe".to_owned(),
        name: "two-steps".to_owned(),
        probes: vec![step(), step()],
        compare: ProbeCompare::default(),
    };
    let options = Options {
        self_check: false,
        paths: Vec::new(),
    };
    assert!(run_case(&options, &commands, &case).await.unwrap());
}

#[rstest]
#[case::smallest(0, 1)]
#[case::half(1_u64 << 52_u32, 1_u32 << 31_u32)]
#[case::largest((1_u64 << 53_u32) - 1, u32::MAX)]
fn a_u32_draw_takes_the_high_bits_of_the_python_draw(#[case] sample: u64, #[case] expected: u32) {
    let samples = [Sample::try_from(sample).unwrap()];
    let mut random = InjectedRandom::from(samples.as_slice());
    assert_eq!(random.try_next_u32().unwrap(), expected);
    assert!(!random.exhausted);
}

#[test]
fn a_u64_draw_scales_the_sample_to_the_full_width() {
    let samples = [Sample::try_from(1_u64 << 52_u32).unwrap()];
    let mut random = InjectedRandom::from(samples.as_slice());
    assert_eq!(random.try_next_u64().unwrap(), (1_u64 << 63_u32) | 2047);
    assert!(!random.exhausted);
}

#[test]
fn a_draw_past_the_samples_answers_the_value_every_range_accepts() {
    let samples: [Sample; 0] = [];
    let mut random = InjectedRandom::from(samples.as_slice());
    assert_eq!(random.try_next_u64().unwrap(), u64::MAX);
    assert_eq!(random.try_next_u32().unwrap(), u32::MAX);
    assert!(random.exhausted);
}

#[tokio::test]
async fn a_probe_that_never_answers_is_stopped_at_the_deadline() {
    let deadline = Duration::from_millis(100);
    let command = ProbeCommand {
        program: PathBuf::from("sleep"),
        args: vec![OsString::from("5")],
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
