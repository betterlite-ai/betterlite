#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser as _;
use serde_json::Value;

use super::capture::{Capture, Run, Sdk, Stop, capture, check_captures};
use super::error::SdkHttpError;
use super::python::{ProcessEnvironment, PythonSdk};
use super::result::{Nullable, SdkResult};
use super::{
    OracleLimits, SdkHttpCase, Side, child, decode_sdk_http_case, decode_sdk_http_result, sides,
};
use crate::case::{Ignore, UpstreamAnswer};
use crate::mock::ProviderRequest;
use crate::process::sync_litellm;
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    sync_litellm(&root).unwrap();
    PythonSdk {
        root,
        inherited: ProcessEnvironment::try_parse_from([env!("CARGO_PKG_NAME")]).unwrap(),
        case_env: case.env.clone(),
        limits: OracleLimits::default(),
    }
}

fn run(
    sdk: &Sdk<'_>,
    case: &SdkHttpCase,
    script: MockScript,
    limits: &OracleLimits,
    cancelled: &AtomicBool,
) -> Capture {
    let run = Run {
        call: &case.call,
        script,
        env: &case.env,
        limits,
        cancelled,
    };
    capture(sdk, run).unwrap()
}

fn run_answers(sdk: &Sdk<'_>, case: &SdkHttpCase, answers: &[UpstreamAnswer]) -> Capture {
    let script = MockScript::try_from(answers).unwrap();
    run(
        sdk,
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

// stands in for the pinned SDK: the real child runner and setpriv launch a script that posts over HTTP
const FAKE_DISPATCH: &str = r#"
import json, os, sys, time, urllib.error, urllib.request

request = json.load(sys.stdin)
log_path = os.environ["FAKE_LOG"]
with open(log_path, "a") as log:
    log.write(request["api_base"] + "\n")
body = json.dumps({"model": "gpt-4o-mini", "messages": [{"role": "user", "content": os.environ["FAKE_CONTENT"]}]}).encode()
answer = None
for _ in range(int(os.environ["FAKE_POSTS"])):
    with open(log_path, "a") as log:
        log.write("posting\n")
    post = urllib.request.Request(
        request["api_base"] + "/chat/completions",
        data=body,
        headers={"content-type": "application/json", "authorization": "Bearer sk-test"},
    )
    try:
        with urllib.request.urlopen(post) as response:
            answer = answer or json.load(response)
    except urllib.error.HTTPError as error:
        error.read()
if os.environ["FAKE_SLEEP"]:
    time.sleep(30)
if answer is None:
    answer = {"id": "chat-empty", "created": 1, "object": "chat.completion", "model": "gpt-4o-mini"}
value = {key: answer[key] for key in ("id", "created", "object", "model")}
value["choices"] = [{"content": None, "finish_reason": None}]
json.dump({"success": {"value": value}}, sys.stdout)
"#;

struct Fake {
    directory: tempfile::TempDir,
    sdk: PythonSdk,
}

impl Fake {
    fn new(case: &SdkHttpCase, posts: usize, content: &str, sleep: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let python = directory
            .path()
            .join("tools/oracle/litellm/.venv/bin/python");
        let dispatch = directory.path().join("tools/oracle/sdk_http/dispatch.py");
        fs::create_dir_all(python.parent().unwrap()).unwrap();
        fs::create_dir_all(dispatch.parent().unwrap()).unwrap();
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("litellm/.venv/bin/python");
        fs::write(
            &python,
            format!("#!/bin/sh\nexec {} \"$@\"\n", real.display()),
        )
        .unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(&dispatch, FAKE_DISPATCH).unwrap();
        let mut sdk = python_sdk(case);
        sdk.root = directory.path().to_owned();
        sdk.case_env = BTreeMap::from([
            (
                "FAKE_LOG".to_owned(),
                directory.path().join("log").display().to_string(),
            ),
            ("FAKE_POSTS".to_owned(), posts.to_string()),
            ("FAKE_CONTENT".to_owned(), content.to_owned()),
            (
                "FAKE_SLEEP".to_owned(),
                if sleep { "1" } else { "" }.to_owned(),
            ),
        ]);
        Self { directory, sdk }
    }

    fn log_path(&self) -> PathBuf {
        self.directory.path().join("log")
    }

    fn api_bases(&self) -> Vec<String> {
        fs::read_to_string(self.log_path())
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("http"))
            .map(ToOwned::to_owned)
            .collect()
    }
}

fn posted_with(
    case: &SdkHttpCase,
    posts: usize,
    content: &str,
    answers: &[UpstreamAnswer],
) -> Capture {
    let fake = Fake::new(case, posts, content, false);
    run_answers(&Sdk::Python(&fake.sdk), case, answers)
}

fn posted(case: &SdkHttpCase, posts: usize, content: &str, id: &str) -> Capture {
    posted_with(case, posts, content, &answering(id))
}

fn succeeded(side: &Capture) -> String {
    match &side.execution.result {
        Ok(SdkResult::Success { value }) => value.id.clone(),
        other => panic!("unexpected result {:?}", other.as_ref().err()),
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
    let native = run_answers(&Sdk::Native, &case, &case.upstream);
    let python = run_answers(&Sdk::Python(&python_sdk(&case)), &case, &case.upstream);
    assert_chat_request(&native);
    assert_chat_request(&python);
    assert_chat_value(&native);
    assert_chat_value(&python);
    judge(&python, &native, &case).unwrap();
    let second = run_answers(&Sdk::Python(&python_sdk(&case)), &case, &case.upstream);
    judge(&python, &second, &case).unwrap();
    assert_case_env_reaches_both_sides();
}

// LiteLLM reads OPENAI_ORGANIZATION for the OpenAI organization, and the native config section reads it through the same lookup
fn assert_case_env_reaches_both_sides() {
    let mut case = load("chat-success.yaml");
    case.env = BTreeMap::from([("OPENAI_ORGANIZATION".to_owned(), "org-env".to_owned())]);
    case.compare
        .upstream_headers
        .push("openai-organization".to_owned());
    let native = run_answers(&Sdk::Native, &case, &case.upstream);
    let python = run_answers(&Sdk::Python(&python_sdk(&case)), &case, &case.upstream);
    for side in [&native, &python] {
        let [request] = side.requests.as_slice() else {
            panic!("expected one provider request")
        };
        assert_eq!(
            request
                .headers
                .get("openai-organization")
                .map(String::as_str),
            Some("org-env")
        );
    }
    judge(&python, &native, &case).unwrap();
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

fn error_body(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn failure_case(status: u16, body: Value) -> SdkHttpCase {
    let mut case = load("chat-success.yaml");
    case.upstream = vec![UpstreamAnswer {
        status,
        headers: BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
        body: Some(body),
        events: None,
    }];
    case.compare.ignore = vec![Ignore {
        pointer: "/failure/message".to_owned(),
        why: "LiteLLM prefixes its exception class and betterlite writes its own text".to_owned(),
    }];
    case
}

fn assert_failure_matches(case: &SdkHttpCase, expected: (&str, u16, Nullable, Nullable, Nullable)) {
    let (class, status, kind, param, code) = expected;
    let native = run_answers(&Sdk::Native, case, &case.upstream);
    let python = run_answers(&Sdk::Python(&python_sdk(case)), case, &case.upstream);
    for side in [&native, &python] {
        assert_eq!(side.requests.len(), 1);
        let Ok(SdkResult::Failure(failure)) = &side.execution.result else {
            panic!(
                "expected a failure: {:?}",
                side.execution.result.as_ref().err()
            )
        };
        assert_eq!((failure.class.as_str(), failure.status), (class, status));
        assert_eq!(
            (&failure.kind, &failure.param, &failure.code),
            (&kind, &param, &code)
        );
    }
    judge(&python, &native, case).unwrap();
}

#[test]
fn sdk_failure_fields() {
    let case = load("chat-authentication-error.yaml");
    let native = run_answers(&Sdk::Native, &case, &case.upstream);
    let python = run_answers(&Sdk::Python(&python_sdk(&case)), &case, &case.upstream);
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
    assert_provider_failures();
}

fn assert_provider_failures() {
    let rate_limit = failure_case(
        429,
        error_body(
            r#"{"error": {"type": "invalid_request_error", "message": "slow down", "param": "requests", "code": "rate_limit_exceeded"}}"#,
        ),
    );
    assert_failure_matches(
        &rate_limit,
        (
            "RateLimitError",
            429,
            Some(Some("throttling_error".to_owned())),
            Some(Some("requests".to_owned())),
            Some(Some("429".to_owned())),
        ),
    );
    let timeout = failure_case(
        504,
        error_body(r#"{"error": {"message": "gateway timeout"}}"#),
    );
    assert_failure_matches(
        &timeout,
        ("Timeout", 504, Some(None), Some(None), Some(None)),
    );
    let api = failure_case(418, error_body(r#"{"error": {"message": "teapot"}}"#));
    assert_failure_matches(&api, ("APIError", 418, Some(None), Some(None), Some(None)));
}

#[test]
fn independent_captures() {
    let case = load("chat-success.yaml");
    assert_separate_captures(&case);
    assert_zero_request_captures(&case);
}

fn assert_separate_captures(case: &SdkHttpCase) {
    let first = posted(case, 1, "hello", "chat_a");
    let second = posted(case, 1, "hello", "chat_b");
    assert_eq!((first.requests.len(), second.requests.len()), (1, 1));
    assert_eq!(
        [succeeded(&first), succeeded(&second)],
        ["chat_a", "chat_b"]
    );
    assert_eq!(
        judge(&first, &second, case).unwrap_err().pointer,
        "/success/value/id"
    );

    let wrong = posted(case, 1, "wrong", "chat_a");
    let difference = judge(&first, &wrong, case).unwrap_err();
    assert_eq!(difference.pointer, "/upstream/0/body/messages/0/content");

    let twice = posted(case, 2, "hello", "chat_a");
    let twice_again = posted(case, 2, "hello", "chat_a");
    assert_eq!(twice.requests.len(), 2);
    assert!(twice.mock_failure.is_some());
    assert_eq!(
        judge(&twice, &twice_again, case).unwrap_err().pointer,
        "/upstream"
    );
    assert_eq!(
        judge(&first, &twice, case).unwrap_err().pointer,
        "/upstream"
    );
}

fn assert_zero_request_captures(case: &SdkHttpCase) {
    let empty = posted_with(case, 0, "hello", &[]);
    let empty_again = posted_with(case, 0, "hello", &[]);
    assert!(empty.requests.is_empty());
    assert_eq!(succeeded(&empty), "chat-empty");
    judge(&empty, &empty_again, case).unwrap();

    let after_empty = posted(case, 1, "hello", "chat_b");
    assert_eq!(succeeded(&after_empty), "chat_b");
    assert_eq!(after_empty.requests.len(), 1);
    assert_eq!(
        judge(&empty, &after_empty, case).unwrap_err().pointer,
        "/success/value/id"
    );
    let one_request = posted(case, 1, "hello", "chat-empty");
    assert_eq!(
        judge(&empty, &one_request, case).unwrap_err().pointer,
        "/upstream/0"
    );
}

fn invalid_case(call: &str, extra: &str) -> Result<SdkHttpCase, SdkHttpError> {
    let text = format!(
        "kind: sdk_http\nname: isolation\nenv: {{}}\n{extra}{call}\nupstream:\n  - status: 200\n    headers: {{}}\n    body: {{id: chat_a, object: chat.completion, created: 1, model: m, choices: []}}\ncompare:\n  headers: []\n  headers_present: []\n  upstream_headers: []\n  ignore: []\n"
    );
    decode_sdk_http_case(text.as_bytes(), &OracleLimits::default())
}

fn env_case(env: &str) -> Result<SdkHttpCase, SdkHttpError> {
    let text = format!(
        "kind: sdk_http\nname: env\nenv: {env}\ncall:\n{VALID_CALL}\nupstream:\n  - status: 200\n    headers: {{}}\n    body: {{id: chat_a, object: chat.completion, created: 1, model: m, choices: []}}\ncompare:\n  headers: []\n  headers_present: []\n  upstream_headers: []\n  ignore: []\n"
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
    assert_rejected_environment();
}

fn assert_rejected_environment() {
    let reserved = [
        "{HTTP_PROXY: 'http://external.test:8080'}",
        "{https_proxy: 'http://external.test:8080'}",
        "{ALL_PROXY: 'http://external.test:8080'}",
        "{no_proxy: '*'}",
        "{PYTHONPATH: /tmp/case-code}",
        "{PATH: /tmp/case-bin}",
        "{HOME: /tmp/case-home}",
        "{UV_INDEX_URL: 'https://external.test/simple'}",
        "{LITELLM_LOCAL_MODEL_COST_MAP: 'False'}",
    ];
    let rejected = reserved.map(|env| env_case(env).expect_err("a reserved variable must fail"));
    assert!(
        rejected
            .iter()
            .all(|error| matches!(error, SdkHttpError::Invalid(_))),
        "{rejected:?}"
    );
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

fn run_normal_mode(fake: &Fake, case: &SdkHttpCase) {
    for side in sides(false) {
        let sdk = Sdk::for_side(side, &fake.sdk);
        run_answers(&sdk, case, &case.upstream)
            .execution
            .result
            .unwrap();
    }
}

#[test]
fn sdk_driver_isolation() {
    let case = invalid_case(&format!("call:\n{VALID_CALL}"), "").unwrap();
    let fake = Fake::new(&case, 1, "hello", false);
    assert_rejected_cases();
    assert!(!fake.directory.path().join("log").exists());

    run_normal_mode(&fake, &case);
    assert_eq!(fake.api_bases().len(), 1);
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
    let sleeping = Fake::new(case, 1, "hello", true);
    let script = MockScript::try_from(case.upstream.as_slice()).unwrap();
    let timed_out = run(
        &Sdk::Python(&sleeping.sdk),
        case,
        script,
        short,
        &AtomicBool::new(false),
    );
    assert!(matches!(
        timed_out.execution.result,
        Err(SdkHttpError::Deadline)
    ));

    let cancelled = AtomicBool::new(false);
    let cancelling = Fake::new(case, 1, "hello", true);
    let cancel_script = MockScript::try_from(case.upstream.as_slice()).unwrap();
    let limits = OracleLimits::default();
    let outcome = thread::scope(|scope| {
        scope.spawn(|| {
            thread::park_timeout(Duration::from_secs(2));
            cancelled.store(true, Ordering::Release);
        });
        run(
            &Sdk::Python(&cancelling.sdk),
            case,
            cancel_script,
            &limits,
            &cancelled,
        )
    });
    assert!(matches!(
        outcome.execution.result,
        Err(SdkHttpError::Cancelled)
    ));
    let api_bases = [sleeping.api_bases(), cancelling.api_bases()].concat();
    assert_eq!(api_bases.len(), 2);
    for api_base in &api_bases {
        assert!(mock_is_closed(api_base));
    }
}

// the mock holds the reply, so only stopping the client and shutting the mock down ends the run early
const HELD_REPLY: &[u8] = br#"{"deadline_ms":30000,"replies":[{"wait":{"delay":{"millis":20000}},"then":{"answer":{"status":200,"headers":{},"body":{"id":"chat_a","object":"chat.completion","created":1,"model":"gpt-4o-mini","choices":[]}}}}]}"#;

fn mock_of(side: &Capture) -> String {
    let [request] = side.requests.as_slice() else {
        panic!("expected one provider request")
    };
    let host = request.headers.get("host").expect("captured host header");
    format!("http://{host}/v1")
}

enum Trigger<'trigger> {
    After(Duration),
    Posted(&'trigger Path),
}

fn cancel_after(cancelled: &AtomicBool, trigger: &Trigger<'_>) {
    match trigger {
        Trigger::After(delay) => thread::park_timeout(*delay),
        Trigger::Posted(log) => {
            let started = Instant::now();
            while !fs::read_to_string(log).is_ok_and(|text| text.contains("posting")) {
                assert!(started.elapsed() < Duration::from_secs(30));
                thread::park_timeout(Duration::from_millis(10));
            }
            thread::park_timeout(Duration::from_millis(300));
        }
    }
    cancelled.store(true, Ordering::Release);
}

fn assert_cancels_mid_reply(
    case: &SdkHttpCase,
    sdk: &Sdk<'_>,
    limits: &OracleLimits,
    trigger: &Trigger<'_>,
) {
    let cancelled = AtomicBool::new(false);
    let started = Instant::now();
    let captured = thread::scope(|scope| {
        scope.spawn(|| cancel_after(&cancelled, trigger));
        run(
            sdk,
            case,
            parse_script(HELD_REPLY).unwrap(),
            limits,
            &cancelled,
        )
    });
    assert!(matches!(
        captured.execution.result,
        Err(SdkHttpError::Cancelled)
    ));
    assert_eq!(captured.requests.len(), 1);
    assert!(mock_is_closed(&mock_of(&captured)));
    assert!(started.elapsed() < Duration::from_secs(10));
}

fn assert_native_stops_at_deadline(case: &SdkHttpCase, short: &OracleLimits) {
    let delayed = br#"{"deadline_ms":30000,"replies":[{"wait":{"delay":{"millis":20000}},"then":{"answer":{"status":200,"headers":{},"body":{"id":"x"}}}}]}"#;
    let started = Instant::now();
    let script = parse_script(delayed).unwrap();
    let native = run(&Sdk::Native, case, script, short, &AtomicBool::new(false));
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
        max_execution: Duration::from_secs(3),
    };
    assert_stops_children_and_mocks(&case, &short);
    assert_native_stops_at_deadline(&case, &short);
    assert_cancels_mid_reply(
        &case,
        &Sdk::Native,
        &limits,
        &Trigger::After(Duration::from_secs(1)),
    );
    let cancelling = Fake::new(&case, 1, "hello", false);
    let log = cancelling.log_path();
    assert_cancels_mid_reply(
        &case,
        &Sdk::Python(&cancelling.sdk),
        &limits,
        &Trigger::Posted(&log),
    );
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

fn spawn_once_running(command: &mut Command, pid_file: &Path) -> Child {
    let child = command.spawn().unwrap();
    let started = Instant::now();
    while !fs::read_to_string(pid_file).is_ok_and(|text| text.ends_with('\n')) {
        assert!(started.elapsed() < Duration::from_secs(30));
        thread::park_timeout(Duration::from_millis(10));
    }
    child
}

// the owner thread ends without killing or waiting, so only setpriv --pdeathsig can stop the child
#[test]
fn child_dies_with_its_owner() {
    let directory = tempfile::tempdir().unwrap();
    let python = directory
        .path()
        .join("tools/oracle/litellm/.venv/bin/python");
    let pid_file = directory.path().join("pid");
    fs::create_dir_all(python.parent().unwrap()).unwrap();
    let script = format!(
        "#!/bin/sh\necho $$ > {}\nexec sleep 30\n",
        pid_file.display()
    );
    fs::write(&python, script).unwrap();
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
    let case = load("chat-success.yaml");
    let mut sdk = python_sdk(&case);
    sdk.root = directory.path().to_owned();
    let mut command = sdk.command();
    let mut child = thread::scope(|scope| {
        scope
            .spawn(|| spawn_once_running(&mut command, &pid_file))
            .join()
            .unwrap()
    });
    let started = Instant::now();
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(9_i32));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!Path::new("/proc").join(child.id().to_string()).exists());
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
