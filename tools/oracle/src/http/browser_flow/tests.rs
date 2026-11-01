#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::io::Error;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::str;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::{env, fs, process};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request as AxumRequest, State};
use axum::http::{Response as AxumResponse, StatusCode};
use betterlite_postgres_fixture::seed::DatabaseFixture;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::{net, runtime};
use tokio_postgres::NoTls;

use super::{CapturedValue, run_case_steps};
use crate::OracleError;
use crate::case::browser_flow::{Capture, CaptureSource, Step};
use crate::case::{BodyKind, Case, Compare, Request, decode_html_form_capture};
use crate::cli::Options;
use crate::http::client;
use crate::paths::unique_id;
use crate::process::{Gateway, GatewaySpec};
use crate::runner::browser_flow_group::compare_steps;
use crate::runner::{DatabaseEnvironment, capture_gateway, run_group_with_url};

#[test]
fn replays_the_browser_form_corpus() {
    let corpus = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/browser_form");
    let seen = fs::read_dir(corpus)
        .expect("browser_form corpus directory exists")
        .map(|entry| {
            let bytes = fs::read(entry.expect("corpus entry is readable").path())
                .expect("corpus file is readable");
            drop(decode_html_form_capture(&bytes));
        })
        .count();
    assert!(seen > 0, "browser_form corpus must not be empty");
}

#[test]
fn redacts_a_secret_capture_from_debug_output() {
    let value = CapturedValue {
        text: SecretString::from("shhh".to_owned()),
        secret: true,
    };
    assert_eq!(format!("{value:?}"), "CapturedValue(redacted)");
}

struct StubAnswer {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

struct StubState {
    answers: Mutex<VecDeque<StubAnswer>>,
    received: Mutex<Vec<(String, BTreeMap<String, String>)>>,
}

struct StubServer {
    base_url: String,
    state: Arc<StubState>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<Result<(), Error>>>,
}

impl StubServer {
    fn start(answers: Vec<StubAnswer>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("stub listener binds");
        listener
            .set_nonblocking(true)
            .expect("stub listener is nonblocking");
        let address = listener.local_addr().expect("stub listener has an address");
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("stub runtime builds");
        let state = Arc::new(StubState {
            answers: Mutex::new(VecDeque::from(answers)),
            received: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .fallback(respond)
            .with_state(Arc::clone(&state));
        let (stop, stopped) = oneshot::channel();
        let shutdown = async {
            let _stopped = stopped.await;
        };
        let thread = thread::Builder::new()
            .spawn(move || {
                runtime.block_on(async move {
                    let socket = net::TcpListener::from_std(listener)?;
                    axum::serve(socket, router)
                        .with_graceful_shutdown(shutdown)
                        .await
                })
            })
            .expect("stub thread spawns");
        Self {
            base_url: format!("http://{address}"),
            state,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    fn received_count(&self) -> usize {
        self.state.received.lock().expect("stub state lock").len()
    }

    fn received(&self, index: usize) -> Option<(String, BTreeMap<String, String>)> {
        self.state
            .received
            .lock()
            .expect("stub state lock")
            .get(index)
            .cloned()
    }

    fn received_path(&self, index: usize) -> Option<String> {
        self.received(index).map(|(path, _headers)| path)
    }

    fn received_header(&self, index: usize, name: &str) -> Option<String> {
        let (_path, headers) = self.received(index)?;
        headers.get(name).cloned()
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _sent = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

async fn respond(
    State(state): State<Arc<StubState>>,
    request: AxumRequest<Body>,
) -> AxumResponse<Body> {
    let path = request
        .uri()
        .path_and_query()
        .map_or_else(String::new, |value| value.as_str().to_owned());
    let headers = request
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                str::from_utf8(value.as_bytes())
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    state
        .received
        .lock()
        .expect("stub state lock")
        .push((path, headers));
    let Some(answer) = state.answers.lock().expect("stub state lock").pop_front() else {
        return AxumResponse::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .expect("stub response builds");
    };
    let mut builder = AxumResponse::builder()
        .status(StatusCode::from_u16(answer.status).expect("stub status is valid"));
    for (name, value) in &answer.headers {
        builder = builder.header(*name, value);
    }
    builder
        .body(Body::from(answer.body))
        .expect("stub response builds")
}

fn answer(status: u16, headers: Vec<(&'static str, String)>, body: &str) -> StubAnswer {
    StubAnswer {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn step(path: &str, headers: BTreeMap<String, String>, captures: Vec<Capture>, form: bool) -> Step {
    Step {
        request: Request {
            method: "GET".to_owned(),
            path: path.to_owned(),
            headers,
            body: serde_json::Value::Null,
            body_kind: BodyKind::Json,
        },
        compare: Compare::default(),
        captures,
        form,
    }
}

fn capture(name: &str, source: CaptureSource) -> Capture {
    Capture {
        name: name.to_owned(),
        source,
        secret: false,
    }
}

fn steps_case(steps: Vec<Step>) -> Case {
    Case {
        name: "cleanup".to_owned(),
        config: String::new(),
        env: BTreeMap::new(),
        native_feature_eligibility: false,
        request: Box::new(Request {
            method: "GET".to_owned(),
            path: "/".to_owned(),
            headers: BTreeMap::new(),
            body: serde_json::Value::Null,
            body_kind: BodyKind::Json,
        }),
        upstream: Vec::new(),
        compare: Box::new(Compare::default()),
        steps: Some(steps),
        database: None,
    }
}

fn case_invalid_message(error: &OracleError) -> &str {
    let OracleError::CaseInvalid { message, .. } = error else {
        panic!("expected a case-invalid error, got {error:?}");
    };
    message
}

fn interpolation_start_captures() -> Vec<Capture> {
    vec![
        capture(
            "cookie_value",
            CaptureSource::Cookie {
                name: "session".to_owned(),
            },
        ),
        capture(
            "header_value",
            CaptureSource::Header {
                name: "x-flow-token".to_owned(),
            },
        ),
        capture(
            "nonce",
            CaptureSource::Body {
                pointer: "/flow/nonce".to_owned(),
            },
        ),
    ]
}

fn interpolation_steps() -> Vec<Step> {
    let start = step(
        "/start",
        BTreeMap::new(),
        interpolation_start_captures(),
        false,
    );
    let next_headers = BTreeMap::from([
        (
            "x-echo".to_owned(),
            "{{capture:header_value:raw}}".to_owned(),
        ),
        (
            "cookie-check".to_owned(),
            "{{capture:cookie_value:form}}".to_owned(),
        ),
    ]);
    let next = step(
        "/next?n={{capture:nonce:query}}",
        next_headers,
        Vec::new(),
        false,
    );
    vec![start, next]
}

#[test]
fn interpolates_captures_into_a_later_step_with_each_encoding() {
    let server = StubServer::start(vec![
        answer(
            200,
            vec![
                ("set-cookie", "session=abc123; Path=/".to_owned()),
                ("x-flow-token", "xyz789".to_owned()),
            ],
            r#"{"flow":{"nonce":"nonceabc7"}}"#,
        ),
        answer(200, vec![], "done"),
    ]);
    let responses = run_case_steps(
        &client().expect("test client builds"),
        &server.base_url,
        &interpolation_steps(),
    )
    .expect("steps run");
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[1].status, 200);
    assert_eq!(
        server.received_header(1, "x-echo"),
        Some("xyz789".to_owned())
    );
    assert_eq!(
        server.received_header(1, "cookie-check"),
        Some("abc123".to_owned())
    );
    assert_eq!(
        server.received_path(1),
        Some("/next?n=nonceabc7".to_owned())
    );
    let cookie_header = server
        .received_header(1, "cookie")
        .expect("jar sends the cookie back");
    assert!(cookie_header.contains("session=abc123"));
}

#[test]
fn detects_a_status_change_introduced_at_the_second_step() {
    let steps = vec![
        step("/one", BTreeMap::new(), Vec::new(), false),
        step("/two", BTreeMap::new(), Vec::new(), false),
    ];
    let first_server =
        StubServer::start(vec![answer(200, vec![], "ok"), answer(200, vec![], "ok")]);
    let second_server = StubServer::start(vec![
        answer(200, vec![], "ok"),
        answer(500, vec![], "broken"),
    ]);
    let client = client().expect("test client builds");
    let first =
        run_case_steps(&client, &first_server.base_url, &steps).expect("first run succeeds");
    let second =
        run_case_steps(&client, &second_server.base_url, &steps).expect("second run succeeds");
    let difference =
        compare_steps(&steps, &first, &second).expect_err("the second step's status differs");
    assert_eq!(difference.pointer, "/steps/1/status");
}

#[test]
fn missing_header_capture_fails_the_case() {
    let server = StubServer::start(vec![answer(200, vec![], "ok")]);
    let steps = vec![step(
        "/start",
        BTreeMap::new(),
        vec![capture(
            "missing",
            CaptureSource::Header {
                name: "x-missing".to_owned(),
            },
        )],
        false,
    )];
    let error = run_case_steps(
        &client().expect("test client builds"),
        &server.base_url,
        &steps,
    )
    .expect_err("a missing capture fails the case");
    assert!(case_invalid_message(&error).contains("did not match any value"));
}

#[test]
fn ambiguous_form_capture_fails_the_case() {
    let html = r#"<html><body><form method="post" action="/login">
        <input name="dup" value="1"><input name="dup" value="2">
        </form></body></html>"#;
    let server = StubServer::start(vec![answer(200, vec![], html)]);
    let steps = vec![step(
        "/login-form",
        BTreeMap::new(),
        vec![capture(
            "picked",
            CaptureSource::Form {
                control: "dup".to_owned(),
            },
        )],
        true,
    )];
    let error = run_case_steps(
        &client().expect("test client builds"),
        &server.base_url,
        &steps,
    )
    .expect_err("an ambiguous form capture fails the case");
    assert!(case_invalid_message(&error).contains("matched more than one value"));
}

#[test]
fn location_query_reads_the_parameter_without_following_the_redirect() {
    let server = StubServer::start(vec![
        answer(
            302,
            vec![("location", "/cb?code=abc&state=xyz".to_owned())],
            "",
        ),
        answer(200, vec![], "ok"),
    ]);
    let steps = vec![
        step(
            "/start",
            BTreeMap::new(),
            vec![capture(
                "code",
                CaptureSource::LocationQuery {
                    name: "code".to_owned(),
                },
            )],
            false,
        ),
        step(
            "/finish?c={{capture:code:query}}",
            BTreeMap::new(),
            Vec::new(),
            false,
        ),
    ];
    let responses = run_case_steps(
        &client().expect("test client builds"),
        &server.base_url,
        &steps,
    )
    .expect("steps run");
    assert_eq!(responses.len(), 2);
    assert_eq!(server.received_count(), 2);
    assert_eq!(server.received_path(1), Some("/finish?c=abc".to_owned()));
}

#[test]
fn jar_and_captures_never_leak_between_separate_proxy_runs() {
    let steps = vec![
        step("/start", BTreeMap::new(), Vec::new(), false),
        step("/next", BTreeMap::new(), Vec::new(), false),
    ];
    let server_with_cookie = StubServer::start(vec![
        answer(
            200,
            vec![("set-cookie", "session=secretA; Path=/".to_owned())],
            "ok",
        ),
        answer(200, vec![], "ok"),
    ]);
    let server_without_cookie =
        StubServer::start(vec![answer(200, vec![], "ok"), answer(200, vec![], "ok")]);
    let client = client().expect("test client builds");
    run_case_steps(&client, &server_with_cookie.base_url, &steps).expect("first run succeeds");
    run_case_steps(&client, &server_without_cookie.base_url, &steps).expect("second run succeeds");
    assert!(
        server_with_cookie
            .received_header(1, "cookie")
            .expect("own cookie is sent back")
            .contains("session=secretA")
    );
    assert_eq!(server_without_cookie.received_header(0, "cookie"), None);
    assert_eq!(server_without_cookie.received_header(1, "cookie"), None);
}

#[test]
fn stops_sending_requests_after_a_capture_fails_mid_transcript() {
    let server = StubServer::start(vec![answer(200, vec![], "ok"), answer(200, vec![], "ok")]);
    let steps = vec![
        step("/one", BTreeMap::new(), Vec::new(), false),
        step(
            "/two",
            BTreeMap::new(),
            vec![capture(
                "missing",
                CaptureSource::Header {
                    name: "x-missing".to_owned(),
                },
            )],
            false,
        ),
        step("/three", BTreeMap::new(), Vec::new(), false),
    ];
    let error = run_case_steps(
        &client().expect("test client builds"),
        &server.base_url,
        &steps,
    )
    .expect_err("a step 2 failure stops the transcript");
    assert!(case_invalid_message(&error).contains("did not match any value"));
    assert_eq!(server.received_count(), 2);
}

#[test]
fn stops_and_cleans_up_the_gateway_after_a_late_step_failure() {
    let server = StubServer::start(vec![answer(200, vec![], "ok"), answer(200, vec![], "ok")]);
    let address: SocketAddr = server
        .base_url
        .strip_prefix("http://")
        .expect("stub base url has an http prefix")
        .parse()
        .expect("stub base url has a valid socket address");
    let log_path = env::temp_dir().join(format!(
        "betterlite-oracle-browser-flow-cleanup-{}.log",
        process::id()
    ));
    let gateway = Gateway::start(GatewaySpec {
        command: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_owned(), "exec sleep 10".to_owned()],
        cwd: env::temp_dir(),
        env: Vec::new(),
        port: address.port(),
        log_path: log_path.clone(),
    })
    .expect("gateway fixture starts");
    let pid = gateway.child.id();
    let steps = vec![
        step("/one", BTreeMap::new(), Vec::new(), false),
        step(
            "/two",
            BTreeMap::new(),
            vec![capture(
                "missing",
                CaptureSource::Header {
                    name: "x-missing".to_owned(),
                },
            )],
            false,
        ),
    ];
    let cases = [steps_case(steps)];
    capture_gateway(&client().expect("test client builds"), gateway, &cases)
        .expect_err("a missing capture at the second step fails the whole gateway capture");
    fs::metadata(format!("/proc/{pid}")).expect_err("the gateway process is gone");
    let _removed = fs::remove_file(&log_path);
}

fn fixture_database_names(url: &str) -> Vec<String> {
    let runtime = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("postgres admin runtime builds");
    runtime.block_on(async {
        let (admin, connection) = tokio_postgres::connect(url, NoTls)
            .await
            .expect("admin connection to postgres succeeds");
        let mut driver = JoinSet::new();
        driver.spawn(connection);
        admin
            .query(
                "select datname from pg_database where datname like 'betterlite_fixture_%'",
                &[],
            )
            .await
            .expect("pg_database query succeeds")
            .into_iter()
            .map(|row| row.get(0))
            .collect()
    })
}

fn fixture_cleanup_steps() -> Vec<Step> {
    vec![
        step("/health/liveliness", BTreeMap::new(), Vec::new(), false),
        step(
            "/health/liveliness",
            BTreeMap::new(),
            vec![capture(
                "missing",
                CaptureSource::Header {
                    name: "x-missing".to_owned(),
                },
            )],
            false,
        ),
    ]
}

fn fresh_oracle_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/oracle")
        .join(unique_id().expect("unique id generates"))
}

#[test]
fn drops_the_fixture_databases_after_a_late_step_failure_with_a_database_fixture() {
    let environment =
        <DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])
            .expect("database environment parses");
    let secret = environment
        .postgres_test_url
        .expect("POSTGRES_TEST_URL is set for this test");
    let mut before = fixture_database_names(secret.expose_secret());
    let mut case = steps_case(fixture_cleanup_steps());
    case.database = Some(DatabaseFixture::default());
    let options = Options {
        self_check: true,
        paths: Vec::new(),
    };
    let error = run_group_with_url(&options, &fresh_oracle_root(), &[case], Some(&secret))
        .expect_err("a missing capture at the second step fails the whole run");
    assert!(matches!(error, OracleError::CaseInvalid { .. }));
    let mut after = fixture_database_names(secret.expose_secret());
    before.sort();
    after.sort();
    assert_eq!(
        before, after,
        "no betterlite_fixture_ database should remain after cleanup"
    );
}
