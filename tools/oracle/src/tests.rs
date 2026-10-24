#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::Error;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use std::{env, process, slice};

#[cfg(unix)]
use std::os::unix::ffi::OsStringExt as _;

use reqwest::blocking::Client;
use serde_json::Value;

use crate::OracleError;
use crate::artifacts;
use crate::case::{Case, Compare, UpstreamAnswer, load_cases};
use crate::cli::Options;
use crate::compare;
use crate::gateway::{self, GatewayInput};
use crate::http::{MAX_BODY, Response, client, ready, send};
use crate::mock::MockProvider;
use crate::mock::ProviderRequest;
use crate::paths;
use crate::process::{Gateway, GatewaySpec, sync_litellm};
use crate::runner;

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
    let name = format!("betterlite-oracle-test-{}-{suffix}.yaml", process::id());
    env::temp_dir().join(name)
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
}

#[rstest::rstest]
#[case::unknown("unknown", "unknown field")]
#[case::missing("missing", "missing field")]
#[case::duplicate("duplicate", "duplicate")]
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
    let error = load_cases(slice::from_ref(&temporary))
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
    assert_eq!(load_cases(slice::from_ref(&directory)).unwrap().len(), 1);
    assert_eq!(load_cases(&[directory.join("case.yaml")]).unwrap().len(), 1);
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn exit_status_maps_success_and_difference() {
    assert_eq!(runner::exit_status(false), process::ExitCode::SUCCESS);
    assert_eq!(runner::exit_status(true), process::ExitCode::from(1));
}

#[test]
fn oracle_error_preserves_typed_sources() {
    let cli_error = <Options as clap::Parser>::try_parse_from(["oracle", "--unknown"])
        .expect_err("an unknown option should fail parsing");
    let cli = OracleError::from(cli_error);
    assert!(matches!(cli, OracleError::Cli(_)));

    let case_error = OracleError::CaseInvalid {
        path: PathBuf::from("case.yaml"),
        message: "invalid".to_owned(),
    };
    assert!(matches!(case_error, OracleError::CaseInvalid { .. }));

    let harness = OracleError::MissingCapture(0);
    assert!(matches!(harness, OracleError::MissingCapture(0)));
}

#[test]
fn oracle_modules_return_the_crate_error() {
    let _: fn(&[PathBuf]) -> Result<Vec<Case>, OracleError> = load_cases;
    let _: fn() -> Result<Client, OracleError> = client;
    let _: fn(&Path) -> Result<(), OracleError> = sync_litellm;
    let _: fn() -> Result<PathBuf, OracleError> = paths::artifact_root;
}

fn wire_response(bytes: Vec<u8>) -> Result<Response, OracleError> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(OracleError::HttpIo)?;
    let address = listener.local_addr().map_err(OracleError::HttpIo)?;
    let client = client()?;
    let server = thread::Builder::new()
        .spawn(move || {
            let (mut stream, _) = listener.accept()?;
            let mut request = [0; 4096];
            if stream.read(&mut request)? == 0 {
                return Err(Error::other("missing request"));
            }
            let _written = stream.write_all(&bytes);
            Ok::<(), Error>(())
        })
        .map_err(OracleError::MockIo)?;
    let result = send(&client, &format!("http://{address}"), &test_case().request);
    server
        .join()
        .map_err(|_panic| OracleError::MockJoin)?
        .map_err(OracleError::MockIo)?;
    result
}

fn mock_response(mock: &MockProvider) -> Result<Response, OracleError> {
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
    serde_saphyr::from_str(&valid_case_yaml()).unwrap()
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
    ready(&client().unwrap(), "http://127.0.0.1:1").unwrap_err();
    let body = "a".repeat(MAX_BODY);
    let wire = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let reply = wire_response(wire.into_bytes()).unwrap();
    assert_eq!(reply.body.as_str().map(str::len), Some(MAX_BODY));
    let oversized = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}a",
        body.len() + 1
    );
    assert!(matches!(
        wire_response(oversized.into_bytes()),
        Err(OracleError::HttpTooLarge)
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
    assert_eq!(&recorded[0].headers["authorization"], "key");
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
    mock.thread = Some(
        thread::Builder::new()
            .spawn(move || {
                let result = worker.join().unwrap();
                thread::park_timeout(Duration::from_millis(50));
                completed.store(true, Ordering::Release);
                result
            })
            .unwrap(),
    );
    drop(mock);
    assert!(joined.load(Ordering::Acquire));
    send(&client().unwrap(), &address, &test_case().request).unwrap_err();
}

#[test]
fn mock_server_handles_nonblocking_waits_and_failure_storage() {
    let mock = MockProvider::start(&[]).unwrap();
    let reply = mock_response(&mock).unwrap();
    assert_eq!(reply.status, 500);
    mock.finish().unwrap_err();
}

fn gateway_fixture(name: &str, script: &str, port: u16) -> Gateway {
    Gateway::start(GatewaySpec {
        command: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_owned(), script.to_owned()],
        cwd: PathBuf::from("/tmp"),
        env: Vec::new(),
        port,
        log_path: temporary_case_path(name),
    })
    .unwrap()
}

#[rstest::rstest]
#[case::litellm(false, true, "litellm.log")]
#[case::betterlite(false, false, "betterlite.log")]
#[case::self_check(true, false, "betterlite.log")]
fn gateway_spec_picks_command_arguments_and_log(
    #[case] self_check: bool,
    #[case] first: bool,
    #[case] log: &str,
) {
    let options = Options {
        self_check,
        paths: Vec::new(),
    };
    let case_env = BTreeMap::from([("CASE".to_owned(), "1".to_owned())]);
    let input = GatewayInput {
        options: &options,
        group_dir: Path::new("/group"),
        config_path: Path::new("/group/config.yaml"),
        case_env: &case_env,
    };
    let spec = gateway::spec(&input, first, 7).unwrap();
    let flags = "--config /group/config.yaml --host 127.0.0.1 --port 7";
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let expected = if self_check || first {
        root.join("tools/oracle/litellm/.venv/bin/litellm")
    } else {
        root.join("target/debug/betterlite")
    };
    assert_eq!(spec.command, expected);
    assert_eq!(spec.args.join(" "), flags);
    assert_eq!(spec.log_path, Path::new("/group").join(log));
    let cost_map = ("LITELLM_LOCAL_MODEL_COST_MAP".to_owned(), "True".to_owned());
    assert_eq!(spec.env.first(), Some(&cost_map));
    assert_eq!(spec.env.last(), Some(&("CASE".to_owned(), "1".to_owned())));
}

#[cfg(unix)]
#[test]
fn gateway_spec_rejects_a_non_utf8_config_path() {
    let options = Options {
        self_check: false,
        paths: Vec::new(),
    };
    let config_path = OsString::from_vec(vec![b'/', 0xff]);
    let case_env = BTreeMap::new();
    let input = GatewayInput {
        options: &options,
        group_dir: Path::new("/group"),
        config_path: Path::new(config_path.as_os_str()),
        case_env: &case_env,
    };
    assert!(gateway::spec(&input, true, 7).is_err());
}

#[test]
fn gateway_spec_keeps_required_environment_without_case_overrides() {
    let options = Options {
        self_check: false,
        paths: Vec::new(),
    };
    let input = GatewayInput {
        options: &options,
        group_dir: Path::new("/group"),
        config_path: Path::new("/group/config.yaml"),
        case_env: &BTreeMap::new(),
    };
    let spec = gateway::spec(&input, false, 7).unwrap();
    assert_eq!(
        spec.env,
        vec![
            ("LITELLM_LOCAL_MODEL_COST_MAP".to_owned(), "True".to_owned()),
            ("LITELLM_MASTER_KEY".to_owned(), "oracle-master".to_owned()),
        ]
    );
}

#[test]
fn process_starts_waits_and_stops_gateways() {
    let mut gateway = gateway_fixture("started", "exec sleep 10", 1);
    assert_eq!(gateway.base_url, "http://127.0.0.1:1");
    assert_eq!(gateway.log_path, temporary_case_path("started"));
    let mock = MockProvider::start(&[answer()]).unwrap();
    gateway.base_url.clone_from(&mock.base_url);
    gateway.wait_ready(&client().unwrap()).unwrap();
    assert_eq!(mock.finish().unwrap()[0].path, "/health/liveliness");
    gateway.base_url = ":".to_owned();
    assert!(matches!(
        gateway.wait_ready(&client().unwrap()),
        Err(OracleError::ProcessReady { .. })
    ));
    gateway.stop().unwrap();
    assert!(gateway.child.try_wait().unwrap().is_some());
    fs::remove_file(&gateway.log_path).unwrap();
    let mut exited = gateway_fixture("exited", "exit 3", 1);
    assert!(matches!(
        exited.wait_ready(&client().unwrap()),
        Err(OracleError::ProcessExited(_))
    ));
    exited.stop().unwrap();
    fs::remove_file(&exited.log_path).unwrap();
    let dropped = gateway_fixture("dropped", "exec sleep 10", 1);
    let pid = dropped.child.id();
    fs::metadata(format!("/proc/{pid}")).unwrap();
    drop(dropped);
    fs::metadata(format!("/proc/{pid}")).unwrap_err();
    fs::remove_file(temporary_case_path("dropped")).unwrap();
}

#[test]
fn litellm_sync_reports_a_missing_project() {
    let error =
        sync_litellm(Path::new("/definitely-missing-betterlite-oracle-project")).unwrap_err();
    assert!(matches!(error, OracleError::ProcessSync(_)));
}

#[test]
fn runner_helpers_create_safe_artifact_paths_and_ports() {
    assert_eq!(paths::safe_name("a/b c-underscore_"), "a_b_c-underscore_");
    assert_eq!(paths::safe_name(""), "case");
    assert!(paths::free_port().unwrap() > 1);
    let first_id = paths::unique_id().unwrap();
    assert!(first_id.starts_with(&format!("{}-", process::id())));
    assert_ne!(first_id, paths::unique_id().unwrap());
}

#[test]
fn run_cases_propagates_a_gateway_start_failure() {
    let options = Options {
        self_check: false,
        paths: Vec::new(),
    };
    let mut case = test_case();
    case.config = "not: [valid".to_owned();
    let result = runner::run_cases(&options, vec![case]);
    let _error = result.unwrap_err();
    fs::remove_dir_all("target/oracle").unwrap();
}

#[test]
fn prepare_writes_the_config_with_the_mock_url() {
    let root = env::temp_dir().join(format!("betterlite-prepare-{}", process::id()));
    let mut case = test_case();
    case.config = "api_base: {{upstream}}/v1".to_owned();
    let (directory, config) = artifacts::prepare(&root, &case, "http://mock").unwrap();
    assert!(directory.starts_with(&root));
    assert_eq!(config, directory.join("config.yaml"));
    let written = fs::read_to_string(config).unwrap();
    assert_eq!(written, "api_base: http://mock/v1");
    fs::remove_dir_all(&root).unwrap();
}

#[rstest::rstest]
#[case::same("same", "null")]
#[case::differ(
    "differ",
    "{\n  \"message\": \"response bodies differ\",\n  \"pointer\": \"/a\"\n}"
)]
fn compare_group_reports_each_case_and_writes_its_artifacts(
    #[case] name: &str,
    #[case] diff: &str,
) {
    let root = env::temp_dir().join(format!("betterlite-group-{}-{name}", process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("first.log"), "first").unwrap();
    fs::write(root.join("second.log"), "second").unwrap();
    let mut same = test_case();
    same.name = "same".to_owned();
    let mut differ = test_case();
    differ.name = "differ".to_owned();
    let captures = runner::GroupCapture {
        first: vec![response(value("{\"a\":1}")), response(value("{\"a\":1}"))],
        second: vec![response(value("{\"a\":1}")), response(value("{\"a\":2}"))],
        first_log: root.join("first.log"),
        second_log: root.join("second.log"),
        requests: Vec::new(),
        midpoint: 0,
    };
    let outcomes = runner::compare_group(&root, &[same, differ], &captures).unwrap();
    assert_eq!(outcomes, vec![false, true]);
    let mut written = fs::read_dir(root.join(name)).unwrap();
    let directory = written.next().unwrap().unwrap().path();
    let read = |file: &str| fs::read_to_string(directory.join(file)).unwrap();
    assert_eq!(read("diff.json"), diff);
    assert_eq!(read("litellm.log"), "first");
    assert_eq!(read("betterlite.log"), "second");
    assert_eq!(read("provider-litellm.json"), "[]");
    assert!(directory.join("client-betterlite.json").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn compare_group_uses_empty_request_slices_for_an_invalid_midpoint() {
    let root = env::temp_dir().join(format!("betterlite-midpoint-{}", process::id()));
    fs::create_dir_all(&root).unwrap();
    let first_log = root.join("first.log");
    let second_log = root.join("second.log");
    fs::write(&first_log, "first").unwrap();
    fs::write(&second_log, "second").unwrap();
    let captures = runner::GroupCapture {
        first: vec![response(value("{}"))],
        second: vec![response(value("{}"))],
        first_log,
        second_log,
        requests: Vec::new(),
        midpoint: 1,
    };
    assert_eq!(
        runner::compare_group(&root, &[test_case()], &captures).unwrap(),
        vec![false]
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn prepare_places_each_group_below_the_artifact_root() {
    let root = env::temp_dir().join(format!("betterlite-prepare-root-{}", process::id()));
    let (directory, config) = artifacts::prepare(&root, &test_case(), "http://mock").unwrap();
    assert_eq!(directory.parent(), Some(root.as_path()));
    assert!(
        directory
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("group-"))
    );
    assert_eq!(config, directory.join("config.yaml"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn artifact_write_reports_a_missing_gateway_log() {
    let root = env::temp_dir().join(format!("betterlite-artifact-error-{}", process::id()));
    fs::create_dir_all(&root).unwrap();
    let missing = root.join("missing.log");
    let case = test_case();
    let first = response(value("{}"));
    let second = response(value("{}"));
    let input = artifacts::ArtifactInput {
        root: &root,
        case: &case,
        first: &first,
        second: &second,
        first_requests: &[],
        second_requests: &[],
        first_log: &missing,
        second_log: &missing,
        result: None,
    };
    let _error = artifacts::write(input).unwrap_err();
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn runner_selects_cases_and_gateways() {
    let request = request(Value::Null);
    assert_eq!(
        runner::request_slice(slice::from_ref(&request), 0, 1).len(),
        1
    );
    assert!(runner::request_slice(slice::from_ref(&request), 1, 1).is_empty());
    let normal = <Options as clap::Parser>::try_parse_from(["oracle", "cases"]).unwrap();
    runner::run_cases(&normal, Vec::new()).unwrap_err();
    let group_root = env::temp_dir().join(format!("betterlite-empty-group-{}", process::id()));
    fs::create_dir_all(&group_root).unwrap();
    runner::run_group(&normal, &group_root, &[]).unwrap_err();
    fs::remove_dir_all(&group_root).unwrap();
    assert!(paths::artifact_root().unwrap().exists());
}

#[test]
fn capture_gateway_collects_http_responses_and_stops_process() {
    let mock = MockProvider::start(&[answer()]).unwrap();
    let address: SocketAddr = mock
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
    fs::metadata(format!("/proc/{pid}")).unwrap_err();
    fs::remove_file(temporary_case_path("capture")).unwrap();
}

#[rstest::rstest]
#[case::json(b"{\"x\":1}", true)]
#[case::invalid_utf8(b"\xff123456", false)]
fn mock_accepts_body_after_headers(#[case] body: &[u8], #[case] valid: bool) {
    let mock = MockProvider::start(&[answer()]).unwrap();
    let mut stream = TcpStream::connect(mock.base_url.strip_prefix("http://").unwrap()).unwrap();
    stream
        .write_all(
            b"POST /x HTTP/1.1\r\nHost: mock\r\nContent-Length: 7\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    thread::park_timeout(Duration::from_millis(50));
    let _sent = stream.write_all(body);
    let mut received = String::new();
    stream.read_to_string(&mut received).unwrap();
    let requests = mock.finish().unwrap();
    if valid {
        assert_eq!(requests[0].body, value("{\"x\":1}"));
    } else {
        assert!(requests.is_empty());
        assert!(received.starts_with("HTTP/1.1 400"));
    }
}
