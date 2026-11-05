#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser as _;
use reqwest::blocking::Client;
use serde_json::Value;

use super::capture::{Capture, Execution, Implementation, Run, Stop, capture, check_captures};
use super::error::SdkHttpError;
use super::native::NativeSdk;
use super::python::{ProcessEnvironment, PythonSdk};
use super::result::{ChatCompletion, Choice, Nullable, SdkResult};
use super::{
    OracleLimits, SdkHttpCall, SdkHttpCase, Side, child, decode_sdk_http_case,
    decode_sdk_http_result, sides,
};
use crate::case::UpstreamAnswer;
use crate::mock::ProviderRequest;
use crate::routing_mock_scripts::{MockScript, parse_script, redact};
use crate::runner::sdk_http::judge;

const CASE_LIMIT: usize = 0x0010_0000;
const CASE_OVER: usize = 0x0010_0001;
const BODY_LIMIT: usize = 0x0100_0000;
const BODY_FITS: usize = 0x00FF_FFFE;
const BODY_OVER: usize = 0x00FF_FFFF;
const BODY_OVER_BYTES: usize = 0x0100_0001;

const ANSWER: &str =
    r#"{"id":"ID","object":"chat.completion","created":1,"model":"gpt-4o-mini","choices":[]}"#;
const VALID_CALL: &str = "  openai_chat: {model: m, messages: [], api_key: k, timeout_seconds: 10}";

fn case_file(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/sdk-http/oracle")
        .join(name);
    fs::read_to_string(path).unwrap()
}

fn load(name: &str) -> SdkHttpCase {
    decode_sdk_http_case(case_file(name).as_bytes(), &OracleLimits::default()).unwrap()
}

fn answering(id: &str) -> Vec<UpstreamAnswer> {
    vec![UpstreamAnswer {
        status: 200,
        headers: BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
        body: Some(serde_json::from_str(&ANSWER.replace("ID", id)).unwrap()),
        events: None,
    }]
}

fn python_sdk(case: &SdkHttpCase) -> PythonSdk {
    PythonSdk {
        root: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        inherited: ProcessEnvironment::try_parse_from([env!("CARGO_PKG_NAME")]).unwrap(),
        case_env: case.env.clone(),
        limits: OracleLimits::default(),
    }
}

fn run(
    implementation: &dyn Implementation,
    case: &SdkHttpCase,
    script: MockScript,
    limits: &OracleLimits,
    cancelled: &AtomicBool,
) -> Capture {
    let run = Run {
        call: &case.call,
        script,
        limits,
        cancelled,
    };
    capture(implementation, run).unwrap()
}

fn run_answers(
    implementation: &dyn Implementation,
    case: &SdkHttpCase,
    answers: &[UpstreamAnswer],
) -> Capture {
    let script = MockScript::try_from(answers).unwrap();
    run(
        implementation,
        case,
        script,
        &OracleLimits::default(),
        &AtomicBool::new(false),
    )
}

fn redacted(value: &str) -> String {
    let mut headers = BTreeMap::from([("authorization".to_owned(), value.to_owned())]);
    redact(&mut headers);
    headers.remove("authorization").unwrap()
}

// posts the chat body the SDK would send, straight over HTTP, to stand in for either implementation
struct Posting {
    posts: usize,
    content: &'static str,
}

impl Implementation for Posting {
    fn execute(&self, _call: &SdkHttpCall, api_base: &str, _stop: &Stop<'_>) -> Execution {
        let content = self.content;
        let body = format!(
            r#"{{"model":"gpt-4o-mini","messages":[{{"role":"user","content":"{content}"}}]}}"#
        );
        let client = Client::new();
        let mut answer = Value::Null;
        for _ in 0..self.posts {
            let response = client
                .post(format!("{api_base}/chat/completions"))
                .header("content-type", "application/json")
                .bearer_auth("sk-test")
                .body(body.clone())
                .send()
                .unwrap();
            if answer.is_null() {
                answer = serde_json::from_slice(&response.bytes().unwrap()).unwrap();
            }
        }
        let text = |name: &str| answer.get(name).and_then(Value::as_str).unwrap().to_owned();
        Execution {
            result: Ok(SdkResult::Success {
                value: ChatCompletion {
                    id: text("id"),
                    created: answer.get("created").and_then(Value::as_u64).unwrap(),
                    object: text("object"),
                    model: Some(text("model")),
                    choices: vec![Choice {
                        content: None,
                        finish_reason: None,
                    }],
                },
            }),
            log: Vec::new(),
        }
    }
}

fn posted(case: &SdkHttpCase, posts: usize, content: &'static str, id: &str) -> Capture {
    run_answers(&Posting { posts, content }, case, &answering(id))
}

struct Counting {
    executions: AtomicUsize,
}

impl Implementation for Counting {
    fn execute(&self, call: &SdkHttpCall, api_base: &str, stop: &Stop<'_>) -> Execution {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Posting {
            posts: 1,
            content: "hello",
        }
        .execute(call, api_base, stop)
    }
}

// runs a child that outlives every test deadline and remembers where the mock listened
struct Sleeping {
    api_bases: Mutex<Vec<String>>,
}

impl Sleeping {
    fn new() -> Self {
        Self {
            api_bases: Mutex::new(Vec::new()),
        }
    }
}

impl Implementation for Sleeping {
    fn execute(&self, _call: &SdkHttpCall, api_base: &str, stop: &Stop<'_>) -> Execution {
        self.api_bases.lock().unwrap().push(api_base.to_owned());
        sleep_until_stopped(stop)
    }
}

fn sleep_until_stopped(stop: &Stop<'_>) -> Execution {
    let piped = child::run(Command::new("sleep").arg("30"), b"", 16, stop);
    Execution {
        result: piped.stdout.and_then(|_bytes| Err(SdkHttpError::Join)),
        log: piped.stderr,
    }
}

fn mock_is_closed(api_base: &str) -> bool {
    let address = api_base
        .trim_start_matches("http://")
        .trim_end_matches("/v1");
    TcpStream::connect(address).is_err()
}

fn text_of(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn assert_chat_request(side: &Capture) {
    let [request] = side.requests.as_slice() else {
        panic!("expected one provider request")
    };
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(
        request.headers.get("authorization"),
        Some(&redacted("Bearer sk-test"))
    );
    assert_eq!(request.body.get("model").map(text_of), Some("gpt-4o-mini"));
    assert_eq!(
        request.body.pointer("/messages/0/content").map(text_of),
        Some("hello")
    );
    assert!(side.mock_failure.is_none());
}

fn assert_chat_value(side: &Capture) {
    let Ok(SdkResult::Success { value }) = &side.execution.result else {
        panic!(
            "expected a success: {:?}",
            side.execution.result.as_ref().err()
        )
    };
    assert_eq!(
        (
            value.id.as_str(),
            value.created,
            value.object.as_str(),
            value.model.as_deref()
        ),
        ("chat_a", 1, "chat.completion", Some("gpt-4o-mini"))
    );
    let [choice] = value.choices.as_slice() else {
        panic!("expected one choice")
    };
    assert_eq!(
        (choice.content.as_deref(), choice.finish_reason.as_deref()),
        (Some("hi"), Some("stop"))
    );
}

#[test]
fn sdk_chat_http() {
    let case = load("chat-success.yaml");
    let native = run_answers(&NativeSdk, &case, &case.upstream);
    let python = run_answers(&python_sdk(&case), &case, &case.upstream);
    assert_chat_request(&native);
    assert_chat_request(&python);
    assert_chat_value(&native);
    assert_chat_value(&python);
    judge(&python, &native, &case).unwrap();
    let second = run_answers(&python_sdk(&case), &case, &case.upstream);
    judge(&python, &second, &case).unwrap();
}

fn assert_authentication_failure(side: &Capture) {
    assert_eq!(side.requests.len(), 1);
    let Ok(SdkResult::Failure(failure)) = &side.execution.result else {
        panic!(
            "expected a failure: {:?}",
            side.execution.result.as_ref().err()
        )
    };
    assert_eq!(
        (failure.class.as_str(), failure.status),
        ("AuthenticationError", 401)
    );
    assert_eq!(
        (&failure.kind, &failure.param, &failure.code),
        (&Some(None), &Some(None), &Some(None))
    );
}

fn decoded_kind(text: &str) -> Nullable {
    match decode_sdk_http_result(text.as_bytes(), &OracleLimits::default()).unwrap() {
        SdkResult::Failure(failure) => failure.kind,
        SdkResult::Success { .. } => None,
    }
}

#[test]
fn sdk_failure_fields() {
    let case = load("chat-authentication-error.yaml");
    let native = run_answers(&NativeSdk, &case, &case.upstream);
    let python = run_answers(&python_sdk(&case), &case, &case.upstream);
    assert_authentication_failure(&native);
    assert_authentication_failure(&python);
    judge(&python, &native, &case).unwrap();
    let mut strict = case;
    strict.compare.ignore.clear();
    let difference = judge(&python, &native, &strict).unwrap_err();
    assert_eq!(difference.pointer, "/failure/message");
    let kinds = [
        decoded_kind(r#"{"failure":{"class":"X","status":400,"message":"m"}}"#),
        decoded_kind(r#"{"failure":{"class":"X","status":400,"type":null,"message":"m"}}"#),
        decoded_kind(r#"{"failure":{"class":"X","status":400,"type":"t","message":"m"}}"#),
    ];
    assert_eq!(kinds, [None, Some(None), Some(Some("t".to_owned()))]);
}

#[test]
fn independent_captures() {
    let case = load("chat-success.yaml");
    let first = posted(&case, 1, "hello", "chat_a");
    let second = posted(&case, 1, "hello", "chat_b");
    assert_eq!((first.requests.len(), second.requests.len()), (1, 1));
    let ids = [&first, &second].map(|side| match &side.execution.result {
        Ok(SdkResult::Success { value }) => value.id.clone(),
        other => panic!("unexpected result {:?}", other.as_ref().err()),
    });
    assert_eq!(ids, ["chat_a", "chat_b"]);
    assert_eq!(
        judge(&first, &second, &case).unwrap_err().pointer,
        "/success/value/id"
    );

    let wrong = posted(&case, 1, "wrong", "chat_a");
    let difference = judge(&first, &wrong, &case).unwrap_err();
    assert_eq!(difference.pointer, "/upstream/0/body/messages/0/content");

    let twice = posted(&case, 2, "hello", "chat_a");
    let twice_again = posted(&case, 2, "hello", "chat_a");
    assert_eq!(twice.requests.len(), 2);
    assert!(twice.mock_failure.is_some());
    assert_eq!(
        judge(&twice, &twice_again, &case).unwrap_err().pointer,
        "/upstream"
    );
    assert_eq!(
        judge(&first, &twice, &case).unwrap_err().pointer,
        "/upstream"
    );
}

fn invalid_case(call: &str, extra: &str) -> Result<SdkHttpCase, SdkHttpError> {
    let text = format!(
        "kind: sdk_http\nname: isolation\nenv: {{}}\n{extra}{call}\nupstream:\n  - status: 200\n    headers: {{}}\n    body: {{id: chat_a, object: chat.completion, created: 1, model: m, choices: []}}\ncompare:\n  headers: []\n  headers_present: []\n  upstream_headers: []\n  ignore: []\n"
    );
    decode_sdk_http_case(text.as_bytes(), &OracleLimits::default())
}

fn assert_rejected_cases() {
    let rejected = [
        invalid_case("call: {python_function: os.system}", ""),
        invalid_case(&format!("call:\n{VALID_CALL}\n{VALID_CALL}"), ""),
        invalid_case(
            "call:\n  openai_chat: {model: m, messages: [], api_key: k, timeout_seconds: 10, timeout: 5}",
            "",
        ),
        invalid_case(
            "call:\n  openai_chat: {model: m, messages: [], api_key: k, timeout_seconds: 10, api_base: https://external.test}",
            "",
        ),
        invalid_case(
            &format!("call:\n{VALID_CALL}"),
            "api_base: https://external.test\n",
        ),
        invalid_case(&format!("call:\n{VALID_CALL}"), "name: again\n"),
    ];
    for outcome in rejected {
        assert!(
            matches!(outcome, Err(SdkHttpError::CaseParse(_))),
            "{outcome:?}"
        );
    }
}

fn assert_python_has_no_proxy_server() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk_http");
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "py") {
            let source = fs::read_to_string(&path).unwrap().to_lowercase();
            assert!(
                !source.contains("fastapi") && !source.contains("litellm.proxy"),
                "{path:?}"
            );
        }
    }
}

fn assert_stdout_separate_from_stderr() {
    let noisy = [
        "-c",
        "echo noise >&2; printf '%s' '{\"success\":{\"value\":{\"id\":\"a\",\"created\":1,\"object\":\"o\",\"model\":null,\"choices\":[]}}}'",
    ];
    let flag = AtomicBool::new(false);
    let stop = Stop {
        deadline: Instant::now().checked_add(Duration::from_secs(30)).unwrap(),
        cancelled: &flag,
    };
    let piped = child::run(Command::new("sh").args(noisy), b"", 4096, &stop);
    decode_sdk_http_result(&piped.stdout.unwrap(), &OracleLimits::default()).unwrap();
    assert_eq!(piped.stderr, b"noise\n");
}

#[test]
fn sdk_driver_isolation() {
    let launched = Counting {
        executions: AtomicUsize::new(0),
    };
    assert_rejected_cases();
    assert_eq!(launched.executions.load(Ordering::SeqCst), 0);

    let case = invalid_case(&format!("call:\n{VALID_CALL}"), "").unwrap();
    run_answers(&launched, &case, &case.upstream);
    assert_eq!(launched.executions.load(Ordering::SeqCst), 1);
    assert_eq!(sides(false), [Side::Python, Side::Native]);
    assert_eq!(sides(true), [Side::Python, Side::Python]);

    assert_python_has_no_proxy_server();
    assert_stdout_separate_from_stderr();
}

fn synthetic(count: usize, body: Value) -> Vec<ProviderRequest> {
    let request = ProviderRequest {
        method: "POST".to_owned(),
        path: "/v1".to_owned(),
        headers: BTreeMap::new(),
        body,
    };
    vec![request; count]
}

fn padded(prefix: &str, size: usize) -> Vec<u8> {
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.resize(size, b' ');
    bytes
}

fn assert_size_limits(limits: &OracleLimits) {
    assert!(matches!(
        decode_sdk_http_case(&padded("kind: sdk_http", CASE_OVER), limits),
        Err(SdkHttpError::CaseTooLarge {
            bytes: CASE_OVER,
            limit: CASE_LIMIT
        })
    ));
    assert!(matches!(
        decode_sdk_http_result(&padded("{}", CASE_OVER), limits),
        Err(SdkHttpError::ResultTooLarge {
            bytes: CASE_OVER,
            limit: CASE_LIMIT
        })
    ));
}

fn assert_capture_limits(limits: &OracleLimits) {
    check_captures(&synthetic(128, Value::Null), limits).unwrap();
    assert!(matches!(
        check_captures(&synthetic(129, Value::Null), limits),
        Err(SdkHttpError::TooManyRequests {
            count: 129,
            limit: 128
        })
    ));
    let at_limit = Value::String("a".repeat(BODY_FITS));
    check_captures(&synthetic(1, at_limit), limits).unwrap();
    let over = Value::String("a".repeat(BODY_OVER));
    assert!(matches!(
        check_captures(&synthetic(1, over), limits),
        Err(SdkHttpError::BodyTooLarge {
            index: 0,
            bytes: BODY_OVER_BYTES,
            limit: BODY_LIMIT
        })
    ));
}

fn assert_stops_children_and_mocks(case: &SdkHttpCase, short: &OracleLimits) {
    let sleeping = Sleeping::new();
    let script = MockScript::try_from(case.upstream.as_slice()).unwrap();
    let timed_out = run(&sleeping, case, script, short, &AtomicBool::new(false));
    assert!(matches!(
        timed_out.execution.result,
        Err(SdkHttpError::Deadline)
    ));

    let cancelled = AtomicBool::new(false);
    let cancelling = Sleeping::new();
    let cancel_script = MockScript::try_from(case.upstream.as_slice()).unwrap();
    let limits = OracleLimits::default();
    let outcome = thread::scope(|scope| {
        scope.spawn(|| {
            thread::park_timeout(Duration::from_millis(200));
            cancelled.store(true, Ordering::Release);
        });
        run(&cancelling, case, cancel_script, &limits, &cancelled)
    });
    assert!(matches!(
        outcome.execution.result,
        Err(SdkHttpError::Cancelled)
    ));
    for api_base in sleeping
        .api_bases
        .lock()
        .unwrap()
        .iter()
        .chain(cancelling.api_bases.lock().unwrap().iter())
    {
        assert!(mock_is_closed(api_base));
    }
}

fn assert_native_stops_at_deadline(case: &SdkHttpCase, short: &OracleLimits) {
    let delayed = br#"{"deadline_ms":30000,"replies":[{"wait":{"delay":{"millis":20000}},"then":{"answer":{"status":200,"headers":{},"body":{"id":"x"}}}}]}"#;
    let started = Instant::now();
    let script = parse_script(delayed).unwrap();
    let native = run(&NativeSdk, case, script, short, &AtomicBool::new(false));
    assert!(matches!(
        native.execution.result,
        Err(SdkHttpError::Deadline)
    ));
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn sdk_limits() {
    let limits = OracleLimits::default();
    assert_size_limits(&limits);
    assert_capture_limits(&limits);
    let case = load("chat-success.yaml");
    let short = OracleLimits {
        max_case_bytes: limits.max_case_bytes,
        max_result_bytes: limits.max_result_bytes,
        max_body_bytes: limits.max_body_bytes,
        max_requests: limits.max_requests,
        max_execution: Duration::from_millis(300),
    };
    assert_stops_children_and_mocks(&case, &short);
    assert_native_stops_at_deadline(&case, &short);
}

#[test]
fn child_is_reaped_at_the_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("pid");
    let flag = AtomicBool::new(false);
    let stop = Stop {
        deadline: Instant::now() + Duration::from_millis(400),
        cancelled: &flag,
    };
    let mut command = Command::new("sh");
    command
        .args(["-c", "echo $$ > \"$1\"; exec sleep 30", "sh"])
        .arg(&pid_file);
    let piped = child::run(&mut command, b"", 16, &stop);
    assert!(matches!(piped.stdout, Err(SdkHttpError::Deadline)));
    let pid = fs::read_to_string(&pid_file).unwrap();
    assert!(!Path::new("/proc").join(pid.trim()).exists());
}

fn replay_corpus(root: &Path, limits: &OracleLimits) -> usize {
    let mut replayed = 0_usize;
    for entry in fs::read_dir(root).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        let (selector, rest) = bytes.split_first().unwrap();
        match selector {
            0 => {
                decode_sdk_http_case(rest, limits).unwrap();
            }
            1 => {
                decode_sdk_http_result(rest, limits).unwrap();
            }
            other => panic!("unknown corpus selector {other}"),
        }
        replayed = replayed.saturating_add(1);
    }
    replayed
}

#[test]
fn replays_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/sdk_http_case");
    assert!(replay_corpus(&root, &OracleLimits::default()) >= 4);
}
