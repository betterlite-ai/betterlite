#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::case::{Case, Compare, UpstreamAnswer, load_cases};
use crate::cli::Options;
use crate::compare;
use crate::http::{HttpError, MAX_BODY, Response, client, ready, send};
use crate::mock::MockProvider;
use crate::mock::ProviderRequest;
use crate::process::{Gateway, GatewaySpec, ProcessError};
use crate::runner::{self, artifacts};

fn response(body: Value) -> Response {
    Response {
        status: 200,
        headers: BTreeMap::new(),
        body,
    }
}

fn request(body: Value) -> ProviderRequest {
    ProviderRequest {
        method: "POST".to_owned(),
        path: "/chat/completions".to_owned(),
        headers: BTreeMap::new(),
        body,
    }
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn valid_case_yaml() -> String {
    "name: test\nconfig: config\nenv: {}\nrequest: {method: GET, path: /, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n".to_owned()
}

fn temporary_case_path(suffix: &str) -> PathBuf {
    let name = format!(
        "betterlite-oracle-test-{}-{suffix}.yaml",
        std::process::id()
    );
    std::env::temp_dir().join(name)
}

#[derive(serde::Deserialize)]
struct Comparison {
    left: Response,
    right: Response,
    left_requests: Vec<ProviderRequest>,
    right_requests: Vec<ProviderRequest>,
    compare: Compare,
    expected: Option<String>,
}

#[rstest::rstest]
#[case::ignored_headers("ignored_headers")]
#[case::nested_pointer("nested_pointer")]
#[case::status("status")]
#[case::headers("headers")]
#[case::missing_left("missing_left")]
#[case::missing_right("missing_right")]
#[case::request_count("request_count")]
#[case::request_method("request_method")]
#[case::request_path("request_path")]
#[case::request_body("request_body")]
#[case::upstream_headers("upstream_headers")]
#[case::escaped_ignore("escaped_ignore")]
#[case::array_ignore("array_ignore")]
#[case::root_ignore("root_ignore")]
#[case::root_pointer("root_pointer")]
#[case::escaped_pointer("escaped_pointer")]
#[case::missing_property("missing_property")]
fn comparison_cases(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/{name}.json"));
    let data: Comparison = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let result = compare::compare(
        &data.left,
        &data.right,
        &data.left_requests,
        &data.right_requests,
        &data.compare,
    );
    assert_eq!(
        result.err().map(|difference| difference.pointer),
        data.expected
    );
    assert_eq!(
        serde_json::to_value(&data.left).unwrap().get("status"),
        Some(&Value::from(data.left.status))
    );
    assert_eq!(
        compare::first_difference_optional(None, None, "/missing"),
        None
    );
}

#[rstest::rstest]
#[case::unknown("unknown", "unknown field")]
#[case::missing("missing", "missing field")]
#[case::duplicate("duplicate", "duplicate field")]
#[case::empty_name("empty_name", "cannot be empty")]
#[case::empty_config("empty_config", "cannot be empty")]
#[case::empty_path("empty_path", "cannot be empty")]
#[case::both_answers("both_answers", "exactly one")]
#[case::no_answer("no_answer", "exactly one")]
#[case::bad_pointer("bad_pointer", "JSON pointers")]
#[case::no_reason("no_reason", "need a why")]
fn invalid_cases(#[case] name: &str, #[case] message: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/{name}.case"));
    let temporary = temporary_case_path(name);
    fs::copy(&path, &temporary).unwrap();
    let error = load_cases(std::slice::from_ref(&temporary))
        .unwrap_err()
        .to_string();
    assert!(error.contains(message), "{error}");
    assert!(error.contains(&temporary.display().to_string()), "{error}");
    fs::remove_file(temporary).unwrap();
}

#[test]
fn case_loads_directories_and_ignores_non_yaml_files() {
    let directory = temporary_case_path("directory");
    fs::create_dir_all(&directory).unwrap();
    let yaml =
        valid_case_yaml().replace("ignore: []", "ignore: [{pointer: '', why: whole document}]");
    fs::write(directory.join("case.yaml"), yaml).unwrap();
    fs::write(directory.join("ignored.txt"), valid_case_yaml()).unwrap();
    assert_eq!(
        load_cases(std::slice::from_ref(&directory)).unwrap().len(),
        1
    );
    assert_eq!(load_cases(&[directory.join("case.yaml")]).unwrap().len(), 1);
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn exit_status_maps_success_and_difference() {
    assert_eq!(crate::exit_status(std::process::ExitCode::SUCCESS), 0);
    assert_eq!(crate::exit_status(std::process::ExitCode::from(1)), 1);
}

fn wire_response(bytes: Vec<u8>) -> Result<Response, HttpError> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        assert!(stream.read(&mut request).unwrap() > 0);
        let _written = stream.write_all(&bytes);
    });
    let result = send(
        &client().unwrap(),
        &format!("http://{address}"),
        &test_case().request,
    );
    server.join().unwrap();
    result
}

fn mock_response(mock: &MockProvider) -> Result<Response, HttpError> {
    let client = client()?;
    send(&client, &mock.base_url, &test_case().request)
}

fn answer() -> UpstreamAnswer {
    UpstreamAnswer {
        status: 200,
        headers: BTreeMap::new(),
        body: Some(value("{}")),
        events: None,
    }
}

fn test_case() -> Case {
    let case = serde_yaml::from_str(&valid_case_yaml()).unwrap();
    case
}

#[rstest::rstest]
#[case::json("json")]
#[case::text("text")]
#[case::stream("stream")]
#[case::chunks("chunks")]
#[case::bad("bad")]
#[case::bad_chunk("bad_chunk")]
#[case::bad_event("bad_event")]
#[case::redirect("redirect")]
fn http_parsers(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/http_{name}.json"));
    let (wire, expected): (String, Option<Value>) =
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(
        wire_response(wire.into_bytes())
            .ok()
            .map(|reply| reply.body),
        expected
    );
    assert_eq!(MAX_BODY, 16 * 1024 * 1024);
}

#[test]
fn http_ready_rejects_non_success_status_and_chunks_allow_the_limit() {
    assert!(ready(&client().unwrap(), "http://127.0.0.1:1").is_err());
    let body = "a".repeat(MAX_BODY);
    let wire = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    assert_eq!(
        wire_response(wire.into_bytes())
            .unwrap()
            .body
            .as_str()
            .unwrap()
            .len(),
        body.len()
    );
    let oversized = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}a",
        body.len() + 1
    );
    assert!(matches!(
        wire_response(oversized.into_bytes()),
        Err(HttpError::TooLarge)
    ));
}

#[test]
fn http_write_request_sends_headers_and_body() {
    let mock = MockProvider::start(&[answer()]).unwrap();
    let mut outgoing = test_case().request;
    outgoing.method = "POST".to_owned();
    outgoing
        .headers
        .insert("authorization".to_owned(), "key".to_owned());
    outgoing.body = value("{\"x\":1}");
    send(&client().unwrap(), &mock.base_url, &outgoing).unwrap();
    let recorded = mock.finish().unwrap();
    assert_eq!(recorded[0].headers.get("authorization").unwrap(), "key");
    assert_eq!(recorded[0].body, outgoing.body);
    assert_eq!(recorded[0].method, "POST");
    assert_eq!(recorded[0].path, "/");
}

#[test]
fn mock_provider_records_requests_and_serves_model_probe() {
    let mut created = answer();
    created.status = 201;
    let mock = MockProvider::start(&[created, answer()]).unwrap();
    let client = client().unwrap();
    let mut outgoing = test_case().request;
    outgoing.path = "/v1/models".to_owned();
    assert_eq!(
        send(&client, &mock.base_url, &outgoing).unwrap().body,
        value(r#"{"object":"list","data":[]}"#)
    );
    assert!(!ready(&client, &mock.base_url).unwrap());
    assert!(ready(&client, &mock.base_url).unwrap());
    assert_eq!(mock.finish().unwrap().len(), 2);
}

#[test]
fn mock_wire_parses_and_writes_http() {
    let mock = MockProvider::start(&[UpstreamAnswer {
        status: 200,
        headers: BTreeMap::from([("content-type".to_owned(), "text/event-stream".to_owned())]),
        body: None,
        events: Some(vec!["{\"event\":true}".to_owned()]),
    }])
    .unwrap();
    let response = mock_response(&mock).unwrap();
    assert_eq!(response.body, value("[{\"event\":true}]"));
    assert_eq!(mock.finish().unwrap().len(), 1);
}

#[test]
fn mock_drop_joins_server() {
    let mut mock = MockProvider::start(&[]).unwrap();
    let address = mock.base_url.clone();
    let worker = mock.thread.take().unwrap();
    let joined = Arc::new(AtomicBool::new(false));
    let completed = Arc::clone(&joined);
    mock.thread = Some(thread::spawn(move || {
        let result = worker.join().unwrap();
        thread::sleep(Duration::from_millis(50));
        completed.store(true, Ordering::Release);
        result
    }));
    drop(mock);
    assert!(joined.load(Ordering::Acquire));
    assert!(send(&client().unwrap(), &address, &test_case().request).is_err());
}

#[test]
fn mock_server_handles_nonblocking_waits_and_failure_storage() {
    let mock = MockProvider::start(&[]).unwrap();
    let reply = mock_response(&mock).unwrap();
    assert_eq!(reply.status, 500);
    assert!(mock.finish().is_err());
    let now = std::time::Instant::now();
    assert!(crate::process::before_deadline(
        now,
        now + Duration::from_secs(1)
    ));
    assert!(!crate::process::before_deadline(now, now));
}

fn gateway_fixture(name: &str, script: &str, port: u16) -> Gateway {
    let log = temporary_case_path(name);
    let args = vec!["-c".to_owned(), script.to_owned()];
    Gateway::start(&GatewaySpec {
        command: Path::new("/bin/sh"),
        args: &args,
        cwd: Path::new("/tmp"),
        env: &[],
        port,
        log_path: &log,
    })
    .unwrap()
}

#[test]
fn process_starts_waits_and_stops_gateways() {
    let mut gateway = gateway_fixture("started", "exec sleep 10", 1);
    assert_eq!(gateway.base_url, "http://127.0.0.1:1");
    assert_eq!(gateway.log_path, temporary_case_path("started"));
    gateway.base_url = ":".to_owned();
    assert!(matches!(
        gateway.wait_ready(&client().unwrap()),
        Err(ProcessError::Ready(_))
    ));
    gateway.stop().unwrap();
    assert!(gateway.child.try_wait().unwrap().is_some());
    fs::remove_file(&gateway.log_path).unwrap();
    let mut exited = gateway_fixture("exited", "exit 3", 1);
    assert!(matches!(
        exited.wait_ready(&client().unwrap()),
        Err(ProcessError::Exited(_))
    ));
    exited.stop().unwrap();
    fs::remove_file(&exited.log_path).unwrap();
    let dropped = gateway_fixture("dropped", "exec sleep 10", 1);
    let pid = dropped.child.id();
    drop(dropped);
    assert!(fs::metadata(format!("/proc/{pid}")).is_err());
    fs::remove_file(temporary_case_path("dropped")).unwrap();
}

#[test]
fn runner_helpers_create_safe_artifact_paths_and_ports() {
    assert_eq!(runner::safe_name("a/b c-underscore_"), "a_b_c-underscore_");
    assert_eq!(runner::safe_name(""), "case");
    let first_port = runner::free_port().unwrap();
    let second_port = runner::free_port().unwrap();
    assert_ne!(first_port, 0);
    assert_ne!(second_port, 0);
    assert_ne!(first_port, 1);
    assert_ne!(second_port, 1);
    assert_ne!(runner::unique_id().unwrap(), 0);
    assert_ne!(runner::unique_id().unwrap(), 1);
    assert_eq!(runner::combine_id(2, 1), 3);
    assert_eq!(runner::combine_id(0, 1), 1);
    assert_eq!(runner::combine_id(2, 3), 1);
    assert!(runner::accumulate_difference(false, true));
    assert!(runner::accumulate_difference(true, false));
    assert!(!runner::accumulate_difference(false, false));
    let request = request(Value::Null);
    assert_eq!(
        runner::request_slice(std::slice::from_ref(&request), 0, 1).len(),
        1
    );
    assert!(runner::request_slice(std::slice::from_ref(&request), 1, 1).is_empty());
    assert_eq!(runner::advance_offset(2, 3), 5);
    let normal = <Options as clap::Parser>::try_parse_from(["oracle", "cases"]).unwrap();
    let self_check =
        <Options as clap::Parser>::try_parse_from(["oracle", "--self-check", "cases"]).unwrap();
    assert!(crate::gateway::uses_litellm(&normal, true));
    assert!(!crate::gateway::uses_litellm(&normal, false));
    assert!(crate::gateway::uses_litellm(&self_check, false));
    assert!(runner::run_cases(&normal, Vec::new()).is_err());
    let group_root =
        std::env::temp_dir().join(format!("betterlite-empty-group-{}", std::process::id()));
    runner::create_dir(&group_root).unwrap();
    assert!(runner::run_group(&normal, &group_root, Vec::new()).is_err());
    fs::remove_dir_all(&group_root).unwrap();
    let directory = std::env::temp_dir().join(format!("betterlite-runner-{}", std::process::id()));
    assert!(runner::artifact_root().unwrap().exists());
    runner::create_dir(&directory).unwrap();
    let file = directory.join("file.txt");
    runner::write_text(&file, "content").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "content");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn artifacts_write_client_provider_diff_and_logs() {
    let root = std::env::temp_dir().join(format!("betterlite-artifacts-{}", std::process::id()));
    let log_directory = root.join("logs");
    fs::create_dir_all(&log_directory).unwrap();
    let first_log = log_directory.join("first.log");
    let second_log = log_directory.join("second.log");
    fs::write(&first_log, "first").unwrap();
    fs::write(&second_log, "second").unwrap();
    let mut case = test_case();
    case.name = "artifact case".to_owned();
    let captured = response(value("{\"ok\":true}"));
    let provider = request(Value::Null);
    let difference = compare::Difference {
        pointer: "/ok".to_owned(),
        message: "response bodies differ".to_owned(),
    };
    let input = artifacts::ArtifactInput {
        root: &root,
        case: &case,
        first: &captured,
        second: &captured,
        first_requests: std::slice::from_ref(&provider),
        second_requests: std::slice::from_ref(&provider),
        first_log: &first_log,
        second_log: &second_log,
        result: Some(&difference),
    };
    let directory = artifacts::write(input).unwrap();
    assert_ne!(directory, artifacts::write(input).unwrap());
    assert!(directory.join("client-litellm.json").exists());
    assert!(directory.join("provider-betterlite.json").exists());
    assert_eq!(
        fs::read_to_string(directory.join("litellm.log")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(directory.join("betterlite.log")).unwrap(),
        "second"
    );
    assert_eq!(
        fs::read_to_string(directory.join("diff.json")).unwrap(),
        "{\n  \"message\": \"response bodies differ\",\n  \"pointer\": \"/ok\"\n}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn capture_gateway_collects_http_responses_and_stops_process() {
    let mock = MockProvider::start(&[answer()]).unwrap();
    let address: std::net::SocketAddr = mock
        .base_url
        .strip_prefix("http://")
        .unwrap()
        .parse()
        .unwrap();
    let gateway = gateway_fixture("capture", "exec sleep 10", address.port());
    let pid = gateway.child.id();
    let captures = runner::capture_gateway(&client().unwrap(), gateway, &[test_case()]).unwrap();
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].status, 200);
    assert_eq!(mock.finish().unwrap().len(), 1);
    assert!(fs::metadata(format!("/proc/{pid}")).is_err());
    fs::remove_file(temporary_case_path("capture")).unwrap();
}

#[test]
fn mock_accepts_body_after_headers() {
    let answer = answer();
    let mock = MockProvider::start(&[answer]).unwrap();
    let mut stream = TcpStream::connect(mock.base_url.strip_prefix("http://").unwrap()).unwrap();
    stream
        .write_all(
            b"POST /x HTTP/1.1\r\nHost: mock\r\nContent-Length: 7\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    thread::sleep(Duration::from_millis(50));
    let _sent = stream.write_all(b"{\"x\":1}");
    let mut received = String::new();
    stream.read_to_string(&mut received).unwrap();
    let requests = mock.finish().unwrap();
    assert_eq!(requests[0].body, value("{\"x\":1}"));
}
