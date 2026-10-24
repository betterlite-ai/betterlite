#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Error, ErrorKind, Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::case::{Case, Compare, Ignore, UpstreamAnswer, load_cases};
use crate::cli::Options;
use crate::compare;
use crate::http::{HttpError, Response};
use crate::mock::ProviderRequest;
use crate::mock::{MockProvider, wire};
use crate::process::{Gateway, GatewaySpec, ProcessError};
use crate::runner;

fn response(body: Value, headers: &[(&str, &str)]) -> Response {
    Response {
        status: 200,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
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
    serde_json::from_str(text).expect("test JSON is valid")
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

#[test]
fn case_rejects_unknown_fields() {
    let yaml = "name: test\nconfig: x\nenv: {}\nrequest: {method: GET, path: /, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\nextra: true\n";
    let error = serde_yaml::from_str::<Case>(yaml).expect_err("unknown field should fail");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn exit_status_maps_success_and_difference() {
    assert_eq!(crate::exit_status(std::process::ExitCode::SUCCESS), 0);
    assert_eq!(crate::exit_status(std::process::ExitCode::from(1)), 1);
}

#[test]
fn case_rejects_missing_fields() {
    let yaml = "name: test\nconfig: x\nenv: {}\nrequest: {method: GET, path: /, headers: {}, body: null}\nupstream: []\n";
    let error = serde_yaml::from_str::<Case>(yaml).expect_err("missing field should fail");
    assert!(error.to_string().contains("missing field"));
}

#[test]
fn case_rejects_duplicate_fields() {
    let yaml = "name: test\nname: other\nconfig: x\nenv: {}\nrequest: {method: GET, path: /, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n";
    let error = serde_yaml::from_str::<Case>(yaml).expect_err("duplicate field should fail");
    assert!(error.to_string().contains("duplicate field"));
}

#[test]
fn comparator_honors_ignore_and_header_rules() {
    let left_response = response(
        value(r#"{"id":"one","stable":"same"}"#),
        &[("content-type", "application/json")],
    );
    let right_response = response(
        value(r#"{"id":"two","stable":"same"}"#),
        &[("content-type", "application/json")],
    );
    let compare = Compare {
        headers: vec!["content-type".to_owned()],
        headers_present: vec!["x-call-id".to_owned()],
        upstream_headers: vec!["authorization".to_owned()],
        ignore: vec![Ignore {
            pointer: "/id".to_owned(),
            why: "generated".to_owned(),
        }],
    };
    let left = Response {
        headers: {
            let mut headers = left_response.headers;
            headers.insert("x-call-id".to_owned(), "one".to_owned());
            headers
        },
        ..left_response
    };
    let right = Response {
        headers: {
            let mut headers = right_response.headers;
            headers.insert("x-call-id".to_owned(), "two".to_owned());
            headers
        },
        ..right_response
    };
    compare::compare(
        &left,
        &right,
        &[request(value(r#"{"x":1}"#))],
        &[request(value(r#"{"x":1}"#))],
        &compare,
    )
    .expect("ignored response fields and selected headers should match");
}

#[test]
fn comparator_reports_first_pointer() {
    let left = response(
        value(r#"{"choices":[{"message":{"content":"left"}}]}"#),
        &[],
    );
    let right = response(
        value(r#"{"choices":[{"message":{"content":"right"}}]}"#),
        &[],
    );
    let error =
        compare::compare(&left, &right, &[], &[], &Compare::default()).expect_err("body differs");
    assert_eq!(error.pointer, "/choices/0/message/content");
}

#[test]
fn case_loads_yaml_and_rejects_invalid_values() {
    let path = temporary_case_path("valid");
    fs::write(&path, valid_case_yaml()).expect("test case should be written");
    let loaded = load_cases(std::slice::from_ref(&path)).expect("valid case should load");
    assert_eq!(loaded.len(), 1);
    fs::remove_file(&path).expect("test case should be removed");

    let invalid_path = temporary_case_path("invalid");
    let invalid = valid_case_yaml().replace("name: test", "name: ''");
    fs::write(&invalid_path, invalid).expect("test case should be written");
    let error = load_cases(std::slice::from_ref(&invalid_path)).expect_err("empty name");
    assert!(error.to_string().contains("cannot be empty"));
    fs::remove_file(&invalid_path).expect("test case should be removed");
}

#[test]
fn case_loads_directories_and_ignores_non_yaml_files() {
    let directory = std::env::temp_dir().join(format!(
        "betterlite-oracle-directory-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("test directory should be created");
    fs::write(directory.join("case.yaml"), valid_case_yaml()).expect("yaml should be written");
    fs::write(directory.join("ignored.txt"), valid_case_yaml()).expect("text should be written");
    let loaded = load_cases(std::slice::from_ref(&directory)).expect("directory should load");
    assert_eq!(loaded.len(), 1);
    fs::remove_dir_all(&directory).expect("test directory should be removed");
}

#[test]
fn case_validation_rejects_bad_upstream_and_ignore_entries() {
    let both = valid_case_yaml().replace(
        "upstream: []",
        "upstream: [{status: 200, headers: {}, body: {}, events: []}]",
    );
    let path = temporary_case_path("both");
    fs::write(&path, both).expect("test case should be written");
    assert!(load_cases(std::slice::from_ref(&path)).is_err());
    fs::remove_file(&path).expect("test case should be removed");

    let bad_pointer =
        valid_case_yaml().replace("ignore: []", "ignore: [{pointer: id, why: generated}]");
    let path = temporary_case_path("pointer");
    fs::write(&path, bad_pointer).expect("test case should be written");
    assert!(load_cases(std::slice::from_ref(&path)).is_err());
    fs::remove_file(&path).expect("test case should be removed");

    let no_answer =
        valid_case_yaml().replace("upstream: []", "upstream: [{status: 200, headers: {}}]");
    let path = temporary_case_path("no-answer");
    fs::write(&path, no_answer).expect("test case should be written");
    assert!(load_cases(std::slice::from_ref(&path)).is_err());
    fs::remove_file(&path).expect("test case should be removed");

    let empty_pointer =
        valid_case_yaml().replace("ignore: []", "ignore: [{pointer: '', why: whole document}]");
    let path = temporary_case_path("empty-pointer");
    fs::write(&path, empty_pointer).expect("test case should be written");
    assert!(load_cases(std::slice::from_ref(&path)).is_ok());
    fs::remove_file(&path).expect("test case should be removed");
}

#[test]
fn case_validation_rejects_empty_config_and_path() {
    let empty_config = valid_case_yaml().replace("config: config", "config: ''");
    let path = temporary_case_path("config");
    fs::write(&path, empty_config).expect("test case should be written");
    assert!(load_cases(std::slice::from_ref(&path)).is_err());
    fs::remove_file(&path).expect("test case should be removed");

    let empty_path = valid_case_yaml().replace("path: /", "path: ''");
    let path = temporary_case_path("path");
    fs::write(&path, empty_path).expect("test case should be written");
    assert!(load_cases(std::slice::from_ref(&path)).is_err());
    fs::remove_file(&path).expect("test case should be removed");
}

#[test]
fn comparator_reports_status_headers_and_provider_differences() {
    let left = response(value("{}"), &[("x", "one")]);
    let right = response(value("{}"), &[("x", "two")]);
    let compare = Compare {
        headers: vec!["x".to_owned()],
        headers_present: Vec::new(),
        upstream_headers: Vec::new(),
        ignore: Vec::new(),
    };
    assert_eq!(
        compare::compare(&left, &right, &[], &[], &compare)
            .expect_err("header mismatch")
            .pointer,
        "/headers/x"
    );

    let compare = Compare {
        headers: Vec::new(),
        headers_present: vec!["x".to_owned()],
        upstream_headers: Vec::new(),
        ignore: Vec::new(),
    };
    let missing = response(value("{}"), &[]);
    assert_eq!(
        compare::compare(&left, &missing, &[], &[], &compare)
            .expect_err("missing header")
            .pointer,
        "/headers/x"
    );

    let status = Response {
        status: 201,
        ..right.clone()
    };
    assert_eq!(
        compare::compare(&left, &status, &[], &[], &Compare::default())
            .expect_err("status mismatch")
            .pointer,
        "/status"
    );

    let provider = request(value("{}"));
    let other_provider = ProviderRequest {
        path: "/other".to_owned(),
        ..provider.clone()
    };
    assert_eq!(
        compare::compare(
            &left,
            &left,
            &[],
            std::slice::from_ref(&provider),
            &Compare::default(),
        )
        .expect_err("request count mismatch")
        .pointer,
        "/upstream"
    );
    assert_eq!(
        compare::compare(
            &left,
            &left,
            std::slice::from_ref(&provider),
            &[other_provider],
            &Compare::default()
        )
        .expect_err("request target mismatch")
        .pointer,
        "/upstream/0"
    );
    let body_provider = request(value("{\"different\":true}"));
    assert_eq!(
        compare::compare(
            &left,
            &left,
            &[provider],
            &[body_provider],
            &Compare::default()
        )
        .expect_err("request body mismatch")
        .pointer,
        "/upstream/0/body"
    );
}

#[test]
fn comparator_handles_json_pointer_escaping_missing_values_and_headers() {
    let left = response(value(r#"{"a/b":{"~key":"left"},"array":[1]}"#), &[]);
    let right = response(value(r#"{"a/b":{"~key":"right"},"array":[1,2]}"#), &[]);
    let ignored = Compare {
        headers: Vec::new(),
        headers_present: Vec::new(),
        upstream_headers: Vec::new(),
        ignore: vec![Ignore {
            pointer: "/a~1b/~0key".to_owned(),
            why: "generated".to_owned(),
        }],
    };
    let error =
        compare::compare(&left, &right, &[], &[], &ignored).expect_err("array length differs");
    assert_eq!(error.pointer, "/array/1");

    let compare = Compare {
        headers: Vec::new(),
        headers_present: Vec::new(),
        upstream_headers: vec!["authorization".to_owned()],
        ignore: Vec::new(),
    };
    let left_request = ProviderRequest {
        headers: BTreeMap::from([(String::from("authorization"), String::from("one"))]),
        ..request(value("{}"))
    };
    let right_request = ProviderRequest {
        headers: BTreeMap::from([(String::from("authorization"), String::from("two"))]),
        ..request(value("{}"))
    };
    assert_eq!(
        compare::compare(&left, &left, &[left_request], &[right_request], &compare)
            .expect_err("provider header mismatch")
            .pointer,
        "/upstream/0/headers/authorization"
    );
    assert_eq!(
        compare::response_document(&left).get("status"),
        Some(&Value::from(200))
    );
    assert_eq!(
        compare::first_difference_optional(None, None, "/missing"),
        None
    );
}

#[test]
fn http_parsers_cover_methods_bodies_sse_and_chunks() {
    assert_eq!(crate::http::MAX_BODY, 16 * 1024 * 1024);
    assert!(crate::http::is_ready_status(200));
    assert!(!crate::http::is_ready_status(500));
    assert!(matches!(
        crate::http::validate_method(""),
        Err(HttpError::Method { .. })
    ));
    assert!(matches!(
        crate::http::validate_method("GET now"),
        Err(HttpError::Method { .. })
    ));
    assert!(matches!(
        crate::http::parse_base("https://127.0.0.1:1"),
        Err(HttpError::Url { .. })
    ));
    assert!(matches!(
        crate::http::parse_base("http://not-a-port"),
        Err(HttpError::Url { .. })
    ));
    assert_eq!(crate::http::validate_method("GET").expect("method"), "GET");

    let json = crate::http::parse_response(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true}",
    )
    .expect("json response");
    assert_eq!(json.body, value("{\"ok\":true}"));
    let text = crate::http::parse_response(b"HTTP/1.1 200 OK\r\n\r\nhello").expect("text response");
    assert_eq!(text.body, Value::String("hello".to_owned()));
    let stream = crate::http::parse_response(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\ndata: {\"x\":1}\n\ndata: [DONE]\n\n",
    ).expect("SSE response");
    assert_eq!(stream.body, value("[{\"x\":1}]"));
    let chunked = crate::http::parse_response(
        b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
    )
    .expect("chunked response");
    assert_eq!(chunked.body, Value::String("hello".to_owned()));
    assert!(crate::http::parse_response(b"bad").is_err());
    assert!(crate::http::decode_chunks(b"x\r\n").is_err());
    assert!(
        crate::http::parse_body(
            &BTreeMap::from([(
                String::from("content-type"),
                String::from("text/event-stream")
            )]),
            b"data: nope\n\n"
        )
        .is_err()
    );
}

#[test]
fn http_ready_rejects_non_success_status_and_chunks_allow_the_limit() {
    let error = crate::http::ready(&crate::http::Client, "http://127.0.0.1:1")
        .expect_err("closed port should fail");
    assert!(matches!(error, HttpError::Connect { .. }));

    let size = crate::http::MAX_BODY;
    let mut chunk = format!("{size:x}\r\n").into_bytes();
    chunk.extend(std::iter::repeat_n(b'a', size));
    chunk.extend_from_slice(b"\r\n0\r\n\r\n");
    let decoded = crate::http::decode_chunks(&chunk).expect("limit-sized chunk");
    assert_eq!(decoded.len(), size);

    let header = b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n";
    let body_size = crate::http::MAX_BODY - header.len();
    let mut exact = header.to_vec();
    exact.extend(std::iter::repeat_n(b'a', body_size));
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("connection");
        stream.write_all(&exact).expect("response");
        stream.shutdown(Shutdown::Both).expect("shutdown");
    });
    let mut client = TcpStream::connect(address).expect("client");
    let response = crate::http::read_response(&mut client).expect("exact limit response");
    assert_eq!(response.status, 200);
    server.join().expect("server should finish");
}

#[test]
fn http_write_request_sends_headers_and_body() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("connection");
        stream.shutdown(Shutdown::Both).expect("shutdown");
    });
    let mut client = TcpStream::connect(address).expect("client");
    thread::sleep(Duration::from_millis(100));
    let error = crate::http::write_request(
        &mut client,
        "POST",
        "/test",
        "127.0.0.1:1",
        &BTreeMap::new(),
        b"{}",
    )
    .expect_err("closed peer should fail the write");
    assert!(matches!(error, HttpError::Io(_)));
    server.join().expect("server should finish");
}

#[test]
fn mock_provider_records_requests_and_serves_model_probe() {
    let answer = UpstreamAnswer {
        status: 200,
        headers: BTreeMap::from([(
            String::from("content-type"),
            String::from("application/json"),
        )]),
        body: Some(value("{\"ok\":true}")),
        events: None,
    };
    let mock = MockProvider::start(std::slice::from_ref(&answer)).expect("mock should start");
    let address = mock.base_url.strip_prefix("http://").expect("mock URL");
    let mut probe = TcpStream::connect(address).expect("probe should connect");
    probe
        .write_all(b"GET /v1/models HTTP/1.1\r\nHost: mock\r\n\r\n")
        .expect("probe should write");
    probe
        .shutdown(Shutdown::Write)
        .expect("probe should finish");
    let mut probe_response = [0_u8; 128];
    let probe_size = probe.read(&mut probe_response).expect("probe response");
    assert!(probe_size > 0);
    let mut request = TcpStream::connect(address).expect("provider should connect");
    request
        .write_all(
            b"POST /chat/completions HTTP/1.1\r\ncontent-length: 2\r\nAuthorization: key\r\n\r\n{}",
        )
        .expect("provider request should write");
    request
        .shutdown(Shutdown::Write)
        .expect("provider request should finish");
    let mut response = [0_u8; 128];
    assert!(request.read(&mut response).expect("provider response") > 0);
    let requests = mock.finish().expect("mock should finish");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/chat/completions");
}

#[test]
fn mock_wire_parses_and_writes_http() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let address = listener.local_addr().expect("address");
    let answer = wire::Answer {
        status: 201,
        headers: BTreeMap::from([(
            String::from("content-type"),
            String::from("application/json"),
        )]),
        body: Some(value("{\"ok\":true}")),
        events: None,
    };
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("connection");
        wire::handle_connection(&mut stream, &answer)
            .expect("wire request")
            .expect("request");
    });
    let mut client = TcpStream::connect(address).expect("client");
    client
        .write_all(b"POST /x HTTP/1.1\r\nContent-Length: 2\r\nX-Test: yes\r\n\r\n{}")
        .expect("request");
    client
        .shutdown(Shutdown::Write)
        .expect("request should finish");
    let mut response = String::new();
    client.read_to_string(&mut response).expect("response");
    server.join().expect("server should finish");
    assert!(response.contains("201"));
    assert!(response.contains("{\"ok\":true}"));

    let events = wire::Answer {
        status: 200,
        headers: BTreeMap::new(),
        body: None,
        events: Some(vec![String::from("{\"event\":true}")]),
    };
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("connection");
        wire::handle_connection(&mut stream, &events).expect("wire request");
    });
    let mut client = TcpStream::connect(address).expect("client");
    client
        .write_all(b"POST /x HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("request");
    client
        .shutdown(Shutdown::Write)
        .expect("request should finish");
    let mut response = String::new();
    client.read_to_string(&mut response).expect("response");
    server.join().expect("server should finish");
    assert!(response.contains("data: {\"event\":true}"));
}

#[test]
fn mock_wire_parses_request_headers_and_limits_reads() {
    let headers =
        wire::request_headers("X-Test: yes\r\nContent-Length: 3\r\n".lines()).expect("headers");
    assert_eq!(headers.0.get("x-test"), Some(&String::from("yes")));
    assert_eq!(headers.1, 3);
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("connection");
        let request = wire::read_request(&mut stream).expect("request");
        assert_eq!(request.body, value("{\"x\":1}"));
    });
    let mut client = TcpStream::connect(address).expect("client");
    client
        .write_all(b"POST /x HTTP/1.1\r\nContent-Length: 7\r\n\r\n{\"x\":1}")
        .expect("request");
    client
        .shutdown(Shutdown::Write)
        .expect("request should finish");
    drop(client);
    server.join().expect("server should finish");
    assert!(
        wire::request_headers("bad".lines())
            .expect("ignored malformed header")
            .0
            .is_empty()
    );
}

#[test]
fn mock_server_handles_nonblocking_waits_and_failure_storage() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let answer = wire::Answer {
        status: 200,
        headers: BTreeMap::new(),
        body: Some(Value::Null),
        events: None,
    };
    let requests = Arc::new(Mutex::new(Vec::new()));
    let failure = Arc::new(Mutex::new(None));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_requests = Arc::clone(&requests);
    let thread_failure = Arc::clone(&failure);
    let thread_stop = Arc::clone(&stop);
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        crate::mock::serve(
            listener,
            vec![answer],
            thread_requests,
            thread_failure,
            thread_stop,
        );
    });
    thread::sleep(Duration::from_millis(20));
    let mut client = TcpStream::connect(address).expect("client");
    client
        .write_all(b"POST /x HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("request");
    client
        .shutdown(Shutdown::Write)
        .expect("request should finish");
    let mut response = [0_u8; 64];
    assert!(client.read(&mut response).expect("response") > 0);
    server.join().expect("server should finish");
    assert_eq!(requests.lock().expect("requests").len(), 1);
    crate::mock::set_failure(&failure, String::from("failed"));
    assert_eq!(failure.lock().expect("failure").as_deref(), Some("failed"));
    assert!(crate::mock::should_serve(false, 0, 1, false));
    assert!(!crate::mock::should_serve(true, 0, 1, false));
    assert!(!crate::mock::should_serve(false, 1, 1, false));
    assert!(!crate::mock::should_serve(false, 0, 1, true));
    assert!(crate::mock::should_report_timeout(false, 0, 1, true));
    assert!(!crate::mock::should_report_timeout(false, 0, 1, false));
    assert!(!crate::mock::should_report_timeout(false, 1, 1, true));
    assert!(crate::mock::should_stop_before_accept(true, false));
    assert!(crate::mock::should_stop_before_accept(false, true));
    assert!(!crate::mock::should_stop_before_accept(false, false));
    assert!(crate::mock::should_stop_after_answer(true, 0, 1));
    assert!(crate::mock::should_stop_after_answer(false, 1, 1));
    assert!(!crate::mock::should_stop_after_answer(false, 0, 1));
    let now = std::time::Instant::now();
    assert!(crate::mock::deadline_reached(now, now));
    assert!(crate::process::before_deadline(
        now,
        now + Duration::from_secs(1)
    ));
    assert!(!crate::process::before_deadline(now, now));
    assert!(wire::should_read_more(0));
    assert!(!wire::should_read_more(wire::MAX_REQUEST));
    assert_eq!(wire::MAX_REQUEST, 16 * 1024 * 1024);
    let probe = ProviderRequest {
        method: String::from("GET"),
        path: String::from("/v1/models"),
        ..request(Value::Null)
    };
    assert!(wire::is_model_probe(&probe));
    let wrong_probe = ProviderRequest {
        path: String::from("/other"),
        ..probe.clone()
    };
    assert!(!wire::is_model_probe(&wrong_probe));
    assert!(wire::header_complete(b"headers\r\n\r\nbody"));
    assert!(!wire::header_complete(b"headers"));
    assert!(crate::mock::should_retry_accept(&Error::from(
        ErrorKind::WouldBlock
    )));
    assert!(!crate::mock::should_retry_accept(&Error::from(
        ErrorKind::Other
    )));
}

#[test]
fn process_starts_waits_and_stops_gateways() {
    let log_path =
        std::env::temp_dir().join(format!("betterlite-gateway-{}.log", std::process::id()));
    let arguments = vec![String::from("-c"), String::from("sleep 10")];
    let specification = GatewaySpec {
        command: Path::new("/bin/sh"),
        args: &arguments,
        cwd: Path::new("/tmp"),
        env: &[],
        port: 1,
        log_path: &log_path,
    };
    let mut gateway = Gateway::start(&specification).expect("gateway should start");
    assert_eq!(gateway.base_url, "http://127.0.0.1:1");
    assert_eq!(gateway.log_path, log_path);
    gateway.stop().expect("gateway should stop");
    assert!(gateway.child.try_wait().expect("child status").is_some());
    fs::remove_file(&log_path).expect("log should be removed");

    let exited_log =
        std::env::temp_dir().join(format!("betterlite-exited-{}.log", std::process::id()));
    let exited_arguments = vec![String::from("-c"), String::from("exit 3")];
    let exited_specification = GatewaySpec {
        command: Path::new("/bin/sh"),
        args: &exited_arguments,
        cwd: Path::new("/tmp"),
        env: &[],
        port: 1,
        log_path: &exited_log,
    };
    let mut exited = Gateway::start(&exited_specification).expect("gateway should start");
    let error = exited
        .wait_ready(&crate::http::Client)
        .expect_err("exited gateway should fail readiness");
    assert!(matches!(error, ProcessError::Exited(_)));
    exited.stop().expect("exited gateway should stop");
    fs::remove_file(&exited_log).expect("log should be removed");

    let dropped_log =
        std::env::temp_dir().join(format!("betterlite-dropped-{}.log", std::process::id()));
    let dropped_arguments = vec![String::from("-c"), String::from("sleep 10")];
    let dropped_specification = GatewaySpec {
        command: Path::new("/bin/sh"),
        args: &dropped_arguments,
        cwd: Path::new("/tmp"),
        env: &[],
        port: 1,
        log_path: &dropped_log,
    };
    let dropped_pid = {
        let dropped = Gateway::start(&dropped_specification).expect("gateway should start");
        dropped.child.id()
    };
    thread::sleep(Duration::from_millis(20));
    assert!(fs::metadata(format!("/proc/{dropped_pid}")).is_err());
    fs::remove_file(&dropped_log).expect("log should be removed");
}

#[test]
fn runner_helpers_create_safe_artifact_paths_and_ports() {
    assert_eq!(runner::safe_name("a/b c-underscore_"), "a_b_c-underscore_");
    assert_eq!(runner::safe_name(""), "case");
    let first_port = runner::free_port().expect("free port");
    let second_port = runner::free_port().expect("free port");
    assert_ne!(first_port, 0);
    assert_ne!(second_port, 0);
    assert_ne!(first_port, 1);
    assert_ne!(second_port, 1);
    assert_ne!(runner::unique_id().expect("unique id"), 0);
    assert_ne!(runner::unique_id().expect("unique id"), 1);
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
    let normal = Options {
        self_check: false,
        paths: Vec::new(),
    };
    let self_check = Options {
        self_check: true,
        paths: Vec::new(),
    };
    assert!(crate::gateway::uses_litellm(&normal, true));
    assert!(!crate::gateway::uses_litellm(&normal, false));
    assert!(crate::gateway::uses_litellm(&self_check, false));
    assert!(runner::run_cases(&normal, Vec::new()).is_err());
    let group_root =
        std::env::temp_dir().join(format!("betterlite-empty-group-{}", std::process::id()));
    runner::create_dir(&group_root).expect("group root");
    assert!(runner::run_group(&normal, &group_root, Vec::new()).is_err());
    fs::remove_dir_all(&group_root).expect("group root should be removed");
    let directory = std::env::temp_dir().join(format!("betterlite-runner-{}", std::process::id()));
    assert!(runner::artifact_root().expect("artifact root").exists());
    runner::create_dir(&directory).expect("directory should be created");
    let file = directory.join("file.txt");
    runner::write_text(&file, "content").expect("file should be written");
    assert_eq!(
        fs::read_to_string(&file).expect("file should be readable"),
        "content"
    );
    fs::remove_dir_all(&directory).expect("directory should be removed");
}

#[test]
fn artifacts_write_client_provider_diff_and_logs() {
    let root = std::env::temp_dir().join(format!("betterlite-artifacts-{}", std::process::id()));
    let log_directory = root.join("logs");
    fs::create_dir_all(&log_directory).expect("log directory");
    let first_log = log_directory.join("first.log");
    let second_log = log_directory.join("second.log");
    fs::write(&first_log, "first").expect("first log");
    fs::write(&second_log, "second").expect("second log");
    let case = Case {
        name: String::from("artifact case"),
        config: String::from("config"),
        env: BTreeMap::new(),
        request: crate::case::Request {
            method: String::from("GET"),
            path: String::from("/"),
            headers: BTreeMap::new(),
            body: Value::Null,
        },
        upstream: Vec::new(),
        compare: Compare::default(),
    };
    let captured = runner::Captured {
        response: response(value("{\"ok\":true}"), &[]),
    };
    let provider = request(Value::Null);
    let difference = compare::compare(
        &captured.response,
        &response(value("{\"ok\":false}"), &[]),
        &[],
        &[],
        &Compare::default(),
    )
    .expect_err("responses should differ");
    runner::artifacts::write(runner::artifacts::ArtifactInput {
        root: &root,
        case: &case,
        first: &captured,
        second: &captured,
        first_requests: std::slice::from_ref(&provider),
        second_requests: std::slice::from_ref(&provider),
        first_log: &first_log,
        second_log: &second_log,
        result: Some(&difference),
    })
    .expect("artifacts should be written");
    let directory = root.join("artifact_case");
    assert!(directory.join("client-litellm.json").exists());
    assert!(directory.join("provider-betterlite.json").exists());
    assert_eq!(
        fs::read_to_string(directory.join("litellm.log")).expect("first log"),
        "first"
    );
    assert_eq!(
        fs::read_to_string(directory.join("betterlite.log")).expect("second log"),
        "second"
    );
    assert_eq!(
        fs::read_to_string(directory.join("diff.json")).expect("diff"),
        "{\n  \"message\": \"response bodies differ\",\n  \"pointer\": \"/ok\"\n}"
    );
    fs::remove_dir_all(&root).expect("artifacts should be removed");
}

#[test]
fn capture_gateway_collects_http_responses_and_stops_process() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let address = listener.local_addr().expect("address");
    let saw_request = Arc::new(AtomicBool::new(false));
    let server_saw_request = Arc::clone(&saw_request);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout");
        let mut request = [0_u8; 512];
        let count = stream.read(&mut request).unwrap_or(0);
        server_saw_request.store(count > 0, std::sync::atomic::Ordering::Relaxed);
        thread::sleep(Duration::from_millis(100));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")
            .expect("response");
        stream.shutdown(Shutdown::Both).expect("shutdown");
    });
    let log_path =
        std::env::temp_dir().join(format!("betterlite-capture-{}.log", std::process::id()));
    let arguments = vec![String::from("-c"), String::from("sleep 10")];
    let specification = GatewaySpec {
        command: Path::new("/bin/sh"),
        args: &arguments,
        cwd: Path::new("/tmp"),
        env: &[],
        port: address.port(),
        log_path: &log_path,
    };
    let gateway = Gateway::start(&specification).expect("gateway should start");
    let case = Case {
        name: String::from("capture"),
        config: String::new(),
        env: BTreeMap::new(),
        request: crate::case::Request {
            method: String::from("GET"),
            path: String::from("/health/liveliness"),
            headers: BTreeMap::new(),
            body: Value::Null,
        },
        upstream: Vec::new(),
        compare: Compare::default(),
    };
    let captures =
        runner::capture_gateway(&crate::http::Client, gateway, &[(PathBuf::new(), case)])
            .expect("gateway should be captured");
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].response.status, 200);
    server.join().expect("server should finish");
    assert!(saw_request.load(std::sync::atomic::Ordering::Relaxed));
    fs::remove_file(&log_path).expect("log should be removed");
}
