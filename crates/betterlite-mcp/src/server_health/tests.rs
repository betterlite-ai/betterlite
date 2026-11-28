#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::mcp::McpStatus;
use betterlite_db::mcp_stored_contracts::MCPTransport;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::error::UserUrlError;
use rstest::rstest;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::openapi::{MAX_SPEC_BYTES, openapi_spec_health};
use super::ports::{ProbeError, SessionError, SystemClock};
use super::settings::{HEALTH_CHECK_TIMEOUT_VARIABLE, HealthSettings};
use crate::config_loader::ports::InstructionsCache;
use crate::openapi_tools::error::OpenApiError;
use crate::server_record::MCPServer;

mod support;

use support::{
    NOW_TEXT, fake_loader, record, rig, rig_with, server_ids, server_json, timestamp_text,
};

const URL: &str = ", \"url\": \"https://up.example/mcp\"";
const SPEC: &str = "https://up.example/spec.json";
const SPEC_EXTRA: &str = ", \"spec_path\": \"https://up.example/spec.json\"";

fn plain(server_id: &str) -> MCPServer {
    record(&server_json(server_id, "http", ""))
}

fn spec_error() -> OpenApiError {
    OpenApiError::Json {
        source: serde_json::from_str::<u8>("\"x\"").unwrap_err(),
    }
}

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl io::Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for CapturedLog {
    type Writer = CapturedLog;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

fn warnings_of<Run>(run: Run) -> String
where
    Run: FnOnce(),
{
    let log = CapturedLog::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(log.clone())
        .finish();
    with_default(subscriber, run);
    let bytes = log.0.lock().unwrap().clone();
    String::from_utf8(bytes).expect("the log is utf-8")
}

#[tokio::test]
async fn check_not_found_row_reports_server_not_found() {
    let a = server_json("a", "http", "");
    let rig = rig(&[a.as_str()], &[]);
    let row = rig.service.check_server("missing").await;
    assert_eq!(row.server_id, "missing");
    assert_eq!(row.server_name, None);
    assert_eq!(row.transport, MCPTransport::Http);
    assert_eq!(row.status, Some(McpStatus::Unknown));
    assert_eq!(row.health_check_error.as_deref(), Some("Server not found"));
    assert_eq!(row.approval_status.as_deref(), Some("active"));
    assert!(!row.is_config.0);
    assert!(row.teams.is_empty());
    assert_eq!(
        timestamp_text(row.last_health_check).as_deref(),
        Some(NOW_TEXT)
    );
    assert!(rig.session.calls().is_empty());
}

#[tokio::test]
async fn check_spec_path_server_takes_the_openapi_probe_result() {
    let spec = server_json("spec", "http", SPEC_EXTRA);
    let rig = rig(&[spec.as_str()], &[]);
    let row = rig.service.check_server("spec").await;
    assert_eq!(row.status, Some(McpStatus::Healthy));
    assert_eq!(row.health_check_error, None);
    assert_eq!(
        timestamp_text(row.last_health_check).as_deref(),
        Some(NOW_TEXT)
    );
    assert_eq!(rig.loader.calls().len(), 1);
    assert!(rig.session.calls().is_empty());
    assert!(row.is_config.0);
    assert_eq!(row.spec_path.as_deref(), Some(SPEC));
}

#[tokio::test]
async fn check_spec_path_failure_keeps_the_probe_message() {
    let spec = server_json("spec", "http", SPEC_EXTRA);
    let rig = rig(&[spec.as_str()], &[]);
    rig.loader.script(Err(OpenApiError::SpecStatus {
        location: SPEC.to_owned(),
        status: 503,
    }));
    let row = rig.service.check_server("spec").await;
    assert_eq!(row.status, Some(McpStatus::Unhealthy));
    assert_eq!(
        row.health_check_error.as_deref(),
        Some("OpenAPI specification request failed (HTTP 503)")
    );
}

#[rstest]
#[case::per_user_auth(", \"auth_type\": \"oauth2\"", true)]
#[case::client_credentials_without_token(
    ", \"auth_type\": \"oauth2\", \"oauth2_flow\": \"client_credentials\"",
    true
)]
#[case::client_credentials_with_token(
    ", \"auth_type\": \"oauth2\", \"oauth2_flow\": \"client_credentials\", \"authentication_token\": \"tok\"",
    false
)]
#[case::missing_token(", \"auth_type\": \"api_key\"", true)]
#[case::empty_token(", \"auth_type\": \"api_key\", \"authentication_token\": \"\"", true)]
#[case::token_present(
    ", \"auth_type\": \"api_key\", \"authentication_token\": \"tok\"",
    false
)]
#[case::aws_sigv4(", \"auth_type\": \"aws_sigv4\"", false)]
#[case::none_auth(", \"auth_type\": \"none\"", false)]
#[case::no_auth("", false)]
#[tokio::test]
async fn check_probe_kind_follows_the_auth_and_token_rules(
    #[case] extra: &str,
    #[case] reachable: bool,
) {
    let json = server_json("a", "http", &format!("{URL}{extra}"));
    let rig = rig(&[json.as_str()], &[]);
    let row = rig.service.check_server("a").await;
    if reachable {
        assert_eq!(row.status, Some(McpStatus::Reachable));
        assert_eq!(rig.probe.calls().len(), 1);
        assert!(rig.session.calls().is_empty());
    } else {
        assert_eq!(row.status, Some(McpStatus::Healthy));
        assert!(rig.probe.calls().is_empty());
        assert_eq!(rig.session.calls().len(), 1);
    }
}

#[tokio::test]
async fn check_per_user_auth_server_skips_the_env_var_check() {
    let json = server_json("a", "http", &format!("{URL}, \"auth_type\": \"oauth2\""));
    let rig = rig(&[json.as_str()], &[]);
    let row = rig.service.check_server("a").await;
    assert_eq!(row.status, Some(McpStatus::Reachable));
    assert!(rig.env_vars.checked().is_empty());
}

#[tokio::test]
async fn check_per_user_env_var_server_uses_reachability() {
    let json = server_json("a", "http", URL);
    let rig = rig(&[json.as_str()], &[]);
    rig.env_vars.mark_per_user("a");
    let row = rig.service.check_server("a").await;
    assert_eq!(row.status, Some(McpStatus::Reachable));
    assert_eq!(rig.probe.calls().len(), 1);
    assert_eq!(rig.env_vars.checked(), vec!["a".to_owned()]);
}

#[tokio::test]
async fn check_session_success_marks_healthy_and_caches_instructions() {
    let json = server_json("a", "http", "");
    let rig = rig(&[json.as_str()], &[]);
    rig.session.script(Ok(Some("  Use it  ".to_owned())));
    let row = rig.service.check_server("a").await;
    assert_eq!(row.status, Some(McpStatus::Healthy));
    assert_eq!(row.health_check_error, None);
    rig.service.ensure_instructions_cached(&plain("a")).await;
    assert_eq!(rig.session.calls().len(), 1);
}

#[tokio::test]
async fn check_session_receives_the_static_headers_and_timeout() {
    let json = server_json("a", "http", ", \"static_headers\": {\"x-api-key\": \"v\"}");
    let rig = rig(&[json.as_str()], &[]);
    let row = rig.service.check_server("a").await;
    assert_eq!(row.status, Some(McpStatus::Healthy));
    let calls = rig.session.calls();
    assert_eq!(calls[0].server_id, "a");
    assert!(calls[0].static_headers);
    assert_eq!(calls[0].timeout, Duration::from_secs(10));
}

#[tokio::test]
async fn check_session_timeout_reports_the_timeout_text() {
    let json = server_json("a", "http", "");
    let rig = rig(&[json.as_str()], &[]);
    rig.session.script(Err(SessionError::Timeout));
    let row = rig.service.check_server("a").await;
    assert_eq!(row.status, Some(McpStatus::Unhealthy));
    assert_eq!(
        row.health_check_error.as_deref(),
        Some("Health check timed out after 10.0 seconds")
    );
}

#[tokio::test]
async fn check_session_cancellation_reports_unknown() {
    let json = server_json("a", "http", "");
    let rig = rig(&[json.as_str()], &[]);
    rig.session.script(Err(SessionError::Cancelled));
    let row = rig.service.check_server("a").await;
    assert_eq!(row.status, Some(McpStatus::Unknown));
    assert_eq!(
        row.health_check_error.as_deref(),
        Some("Health check was cancelled")
    );
}

#[tokio::test]
async fn check_session_failure_carries_the_error_text() {
    let json = server_json("a", "http", "");
    let rig = rig(&[json.as_str()], &[]);
    rig.session
        .script(Err(SessionError::Failed("upstream is down".to_owned())));
    let row = rig.service.check_server("a").await;
    assert_eq!(row.status, Some(McpStatus::Unhealthy));
    assert_eq!(row.health_check_error.as_deref(), Some("upstream is down"));
}

#[tokio::test]
async fn check_cancelled_service_reports_unknown() {
    let json = server_json("a", "http", "");
    let rig = rig(&[json.as_str()], &[]);
    rig.cancel.cancel();
    let row = rig.service.check_server("a").await;
    assert_eq!(row.status, Some(McpStatus::Unknown));
    assert_eq!(
        row.health_check_error.as_deref(),
        Some("Health check was cancelled")
    );
    assert!(rig.session.calls().is_empty());
}

#[rstest]
#[case::absent(None, Duration::from_secs(10))]
#[case::empty(Some(""), Duration::from_secs(10))]
#[case::blank(Some("   "), Duration::from_secs(10))]
#[case::fraction(Some("2.5"), Duration::from_millis(2_500))]
#[case::whole(Some("30"), Duration::from_secs(30))]
#[case::words(Some("soon"), Duration::from_secs(10))]
#[case::zero(Some("0"), Duration::from_secs(10))]
#[case::negative(Some("-1"), Duration::from_secs(10))]
#[case::infinite(Some("1e999"), Duration::from_secs(10))]
fn check_timeout_setting_reads_the_environment(
    #[case] value: Option<&str>,
    #[case] expected: Duration,
) {
    let mut environment = BTreeMap::new();
    if let Some(text) = value {
        environment.insert(HEALTH_CHECK_TIMEOUT_VARIABLE.to_owned(), text.to_owned());
    }
    let settings = HealthSettings::from_lookup(&environment);
    assert_eq!(settings.health_check_timeout, expected);
}

#[test]
fn check_timeout_setting_warns_on_a_non_blank_invalid_value() {
    let warned = warnings_of(|| {
        let environment =
            BTreeMap::from([(HEALTH_CHECK_TIMEOUT_VARIABLE.to_owned(), "soon".to_owned())]);
        let settings = HealthSettings::from_lookup(&environment);
        assert_eq!(settings.health_check_timeout, Duration::from_secs(10));
    });
    assert!(warned.contains("not a positive number of seconds"));
    let quiet = warnings_of(|| {
        let environment =
            BTreeMap::from([(HEALTH_CHECK_TIMEOUT_VARIABLE.to_owned(), "   ".to_owned())]);
        let settings = HealthSettings::from_lookup(&environment);
        assert_eq!(settings.health_check_timeout, Duration::from_secs(10));
    });
    assert!(!quiet.contains("not a positive number of seconds"));
}

#[tokio::test]
async fn check_timeout_setting_reaches_the_probe_and_the_message() {
    let json = server_json("a", "http", "");
    let settings = HealthSettings {
        health_check_timeout: Duration::from_millis(1_500),
    };
    let rig = rig_with(&[json.as_str()], &[], settings);
    rig.session.script(Err(SessionError::Timeout));
    let row = rig.service.check_server("a").await;
    assert_eq!(
        row.health_check_error.as_deref(),
        Some("Health check timed out after 1.5 seconds")
    );
    assert_eq!(rig.session.calls()[0].timeout, Duration::from_millis(1_500));
}

#[tokio::test]
async fn check_system_clock_moves_forward() {
    let clock = SystemClock::default();
    let monotonic_before = clock.now_monotonic();
    let utc_before = clock.now_utc();
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(clock.now_monotonic() > monotonic_before);
    assert!(clock.now_utc() > utc_before);
}

#[tokio::test]
async fn row_missing_lists_are_empty_and_oauth_urls_fall_back() {
    let json = server_json(
        "a",
        "http",
        ", \"authorization_url\": \"https://auth.example/a\", \"token_url\": \"https://auth.example/t\", \"registration_url\": \"https://auth.example/r\"",
    );
    let rig = rig(&[], &[json.as_str()]);
    let row = rig.service.check_server("a").await;
    assert!(row.teams.is_empty());
    assert!(row.mcp_access_groups.is_empty());
    assert!(row.allowed_tools.is_empty());
    assert!(row.extra_headers.is_empty());
    assert!(row.args.is_empty());
    assert_eq!(
        row.authorization_url.as_deref(),
        Some("https://auth.example/a")
    );
    assert_eq!(row.token_url.as_deref(), Some("https://auth.example/t"));
    assert_eq!(
        row.registration_url.as_deref(),
        Some("https://auth.example/r")
    );
}

#[rstest]
#[case::configured(
    ", \"authorization_url\": \"https://live.example/a\", \"configured_authorization_url\": \"https://cfg.example/a\"",
    Some("https://cfg.example/a")
)]
#[case::live(
    ", \"authorization_url\": \"https://live.example/a\"",
    Some("https://live.example/a")
)]
#[case::absent("", None)]
#[tokio::test]
async fn row_authorization_url_prefers_the_configured_value(
    #[case] extra: &str,
    #[case] expected: Option<&str>,
) {
    let json = server_json("a", "http", extra);
    let rig = rig(&[], &[json.as_str()]);
    let row = rig.service.check_server("a").await;
    assert_eq!(row.authorization_url.as_deref(), expected);
}

#[tokio::test]
async fn row_carries_the_server_fields() {
    let json = server_json(
        "a",
        "sse",
        ", \"url\": \"https://up.example/mcp\", \"server_name\": \"upstream\", \"alias\": \"al\", \"access_groups\": [\"g1\"], \"allowed_tools\": [\"tool\"], \"extra_headers\": [\"x-a\"], \"mcp_info\": {\"description\": \"Desc\"}, \"instructions\": \"Use it\"",
    );
    let rig = rig(&[], &[json.as_str()]);
    let row = rig.service.check_server("a").await;
    assert_eq!(row.server_id, "a");
    assert_eq!(row.server_name.as_deref(), Some("upstream"));
    assert_eq!(row.alias.as_deref(), Some("al"));
    assert_eq!(row.description.as_deref(), Some("Desc"));
    assert_eq!(row.mcp_access_groups, vec!["g1".to_owned()]);
    assert_eq!(row.allowed_tools, vec!["tool".to_owned()]);
    assert_eq!(row.extra_headers, vec!["x-a".to_owned()]);
    assert_eq!(row.instructions.as_deref(), Some("Use it"));
    assert_eq!(row.transport, MCPTransport::Sse);
    assert!(!row.is_config.0);
    assert_eq!(row.status, Some(McpStatus::Healthy));
}

#[tokio::test]
async fn row_session_result_takes_the_pydantic_defaults() {
    let json = server_json(
        "a",
        "http",
        ", \"spec_path\": \"\", \"is_byok\": true, \"byok_description\": [\"why\"], \"byok_api_key_help_url\": \"https://help.example\", \"source_url\": \"https://src.example\", \"available_on_public_internet\": false, \"delegate_auth_to_upstream\": true, \"oauth_passthrough\": true, \"configured_scopes\": [\"s1\"], \"tool_name_to_display_name\": {\"t\": \"T\"}",
    );
    let rig = rig(&[], &[json.as_str()]);
    let row = rig.service.check_server("a").await;
    assert!(row.available_on_public_internet.0);
    assert!(!row.delegate_auth_to_upstream.0);
    assert!(!row.oauth_passthrough.0);
    assert!(!row.is_byok.0);
    assert!(row.byok_description.is_empty());
    assert_eq!(row.byok_api_key_help_url, None);
    assert_eq!(row.source_url, None);
    assert!(row.credentials.is_none());
    assert_eq!(row.spec_path, None);
    assert_eq!(row.tool_name_to_display_name, None);
    assert_eq!(row.pinned_tools, None);
}

#[tokio::test]
async fn row_spec_path_result_keeps_the_table_view() {
    let json = server_json(
        "a",
        "http",
        ", \"spec_path\": \"https://up.example/spec.json\", \"is_byok\": true, \"byok_description\": [\"why\"], \"configured_scopes\": [\"s1\"], \"tool_name_to_display_name\": {\"t\": \"T\"}, \"available_on_public_internet\": false",
    );
    let rig = rig(&[json.as_str()], &[]);
    let row = rig.service.check_server("a").await;
    assert!(row.is_config.0);
    assert!(row.is_byok.0);
    assert!(!row.available_on_public_internet.0);
    assert_eq!(row.byok_description, vec!["why".to_owned()]);
    assert!(row.credentials.is_some());
    assert!(row.tool_name_to_display_name.is_some());
    assert_eq!(row.status, Some(McpStatus::Healthy));
}

#[rstest]
#[case::stdio("stdio", "", "Server reachability requires an HTTP or SSE URL")]
#[case::no_url("http", "", "Server reachability requires an HTTP or SSE URL")]
#[case::empty_url(
    "http",
    ", \"url\": \"\"",
    "Server reachability requires an HTTP or SSE URL"
)]
#[case::unparseable(
    "http",
    ", \"url\": \"not a url\"",
    "Server reachability requires an HTTP URL without embedded credentials"
)]
#[case::ftp(
    "http",
    ", \"url\": \"ftp://up.example/mcp\"",
    "Server reachability requires an HTTP URL without embedded credentials"
)]
#[case::userinfo(
    "http",
    ", \"url\": \"https://user:secret@up.example/mcp\"",
    "Server reachability requires an HTTP URL without embedded credentials"
)]
#[case::user_without_password(
    "http",
    ", \"url\": \"https://user@up.example/mcp\"",
    "Server reachability requires an HTTP URL without embedded credentials"
)]
#[case::password_without_user(
    "http",
    ", \"url\": \"https://:secret@up.example/mcp\"",
    "Server reachability requires an HTTP URL without embedded credentials"
)]
#[tokio::test]
async fn reach_refuses_a_url_it_cannot_probe(
    #[case] transport: &str,
    #[case] extra: &str,
    #[case] expected: &str,
) {
    let json = server_json("a", transport, extra);
    let rig = rig(&[json.as_str()], &[]);
    let (status, error) = rig
        .service
        .reachability(&record(&json), Duration::from_secs(10))
        .await;
    assert_eq!(status, McpStatus::Unknown);
    assert_eq!(error.as_deref(), Some(expected));
    assert!(rig.probe.calls().is_empty());
}

#[rstest]
#[case::http("http")]
#[case::sse("sse")]
#[tokio::test]
async fn reach_success_reports_reachable_with_the_accept_header(#[case] transport: &str) {
    let json = server_json("a", transport, URL);
    let rig = rig(&[json.as_str()], &[]);
    let (status, error) = rig
        .service
        .reachability(&record(&json), Duration::from_secs(7))
        .await;
    assert_eq!(status, McpStatus::Reachable);
    assert_eq!(error, None);
    let calls = rig.probe.calls();
    assert_eq!(calls[0].url, "https://up.example/mcp");
    assert_eq!(calls[0].accept, "text/event-stream, application/json");
    assert_eq!(calls[0].timeout, Duration::from_secs(7));
}

#[rstest]
#[case::timeout(
    Err(ProbeError::Timeout),
    McpStatus::Unhealthy,
    Some("Reachability check timed out after 10.0 seconds")
)]
#[case::failed(
    Err(ProbeError::Failed("ConnectError".to_owned())),
    McpStatus::Unhealthy,
    Some("Reachability check failed (ConnectError)")
)]
#[tokio::test]
async fn reach_failures_report_the_status_and_message(
    #[case] outcome: Result<(), ProbeError>,
    #[case] status: McpStatus,
    #[case] message: Option<&str>,
) {
    let json = server_json("a", "http", URL);
    let rig = rig(&[json.as_str()], &[]);
    rig.probe.script(outcome);
    let (reported, error) = rig
        .service
        .reachability(&record(&json), Duration::from_secs(10))
        .await;
    assert_eq!(reported, status);
    assert_eq!(error.as_deref(), message);
}

#[tokio::test]
async fn reach_cancellation_reports_unknown() {
    let json = server_json("a", "http", URL);
    let rig = rig(&[json.as_str()], &[]);
    rig.cancel.cancel();
    let (status, error) = rig
        .service
        .reachability(&record(&json), Duration::from_secs(10))
        .await;
    assert_eq!(status, McpStatus::Unknown);
    assert_eq!(error.as_deref(), Some("Reachability check was cancelled"));
    assert!(rig.probe.calls().is_empty());
}

#[tokio::test]
async fn openapi_local_spec_reports_no_protocol_probe() {
    let (loader, log) = fake_loader();
    let (status, error) =
        openapi_spec_health(&loader, "/specs/a.json", Duration::from_secs(10)).await;
    assert_eq!(status, McpStatus::Unknown);
    assert_eq!(
        error.as_deref(),
        Some("OpenAPI servers have no protocol-level health probe")
    );
    assert!(log.calls().is_empty());
}

#[tokio::test]
async fn openapi_success_reports_healthy_and_limits_the_spec_size() {
    let (loader, log) = fake_loader();
    let (status, error) = openapi_spec_health(&loader, SPEC, Duration::from_secs(10)).await;
    assert_eq!(status, McpStatus::Healthy);
    assert_eq!(error, None);
    let calls = log.calls();
    assert_eq!(calls[0].spec_path, SPEC);
    assert_eq!(calls[0].max_bytes, MAX_SPEC_BYTES);
}

#[rstest]
#[case::status(
    OpenApiError::SpecStatus { location: SPEC.to_owned(), status: 503 },
    McpStatus::Unhealthy,
    "OpenAPI specification request failed (HTTP 503)"
)]
#[case::parse(
    spec_error(),
    McpStatus::Unhealthy,
    "OpenAPI specification could not be loaded (JSONDecodeError)"
)]
#[case::missing(
    OpenApiError::SpecNotFound { location: SPEC.to_owned() },
    McpStatus::Unhealthy,
    "OpenAPI specification could not be loaded (FileNotFoundError)"
)]
#[case::read(
    OpenApiError::SpecRead { location: SPEC.to_owned(), source: io::Error::other("boom") },
    McpStatus::Unhealthy,
    "OpenAPI specification could not be loaded (OSError)"
)]
#[case::fetch(
    OpenApiError::SpecFetch { location: SPEC.to_owned(), source: UserUrlError::TooManyRedirects },
    McpStatus::Unhealthy,
    "OpenAPI specification could not be loaded (RequestError)"
)]
#[case::body(OpenApiError::SpecBody { location: SPEC.to_owned(), source: TransportError::Closed }, McpStatus::Unhealthy, "OpenAPI specification could not be loaded (ReadError)")]
#[tokio::test]
async fn openapi_load_failures_name_the_error_kind(
    #[case] outcome: OpenApiError,
    #[case] status: McpStatus,
    #[case] message: &str,
) {
    let (loader, log) = fake_loader();
    log.script(Err(outcome));
    let (reported, error) = openapi_spec_health(&loader, SPEC, Duration::from_secs(10)).await;
    assert_eq!(reported, status);
    assert_eq!(error.as_deref(), Some(message));
}

#[tokio::test]
async fn openapi_size_limit_is_refused() {
    let (loader, log) = fake_loader();
    log.script(Err(OpenApiError::SpecBody {
        location: SPEC.to_owned(),
        source: TransportError::BodyTooLarge {
            limit: MAX_SPEC_BYTES,
        },
    }));
    let (status, error) = openapi_spec_health(&loader, SPEC, Duration::from_secs(10)).await;
    assert_eq!(status, McpStatus::Unknown);
    assert_eq!(
        error.as_deref(),
        Some(
            "OpenAPI specification probe refused: the response body is larger than 10485760 bytes"
        )
    );
}

#[tokio::test(start_paused = true)]
async fn openapi_timeout_reports_unhealthy() {
    let (loader, log) = fake_loader();
    log.hold_open();
    let (status, error) = openapi_spec_health(&loader, SPEC, Duration::from_secs(10)).await;
    assert_eq!(status, McpStatus::Unhealthy);
    assert_eq!(
        error.as_deref(),
        Some("OpenAPI specification check timed out after 10.0 seconds")
    );
    assert_eq!(log.calls().len(), 1);
}

#[tokio::test]
async fn openapi_result_is_reused_within_thirty_seconds() {
    let rig = rig(&[], &[]);
    let first = rig.service.spec_health(SPEC).await;
    assert_eq!(first.status, McpStatus::Healthy);
    assert_eq!(rig.loader.calls().len(), 1);
    rig.clock.advance(Duration::from_millis(29_900));
    let cached = rig.service.spec_health(SPEC).await;
    assert_eq!(cached, first);
    assert_eq!(rig.loader.calls().len(), 1);
    rig.clock.advance(Duration::from_millis(100));
    let fresh = rig.service.spec_health(SPEC).await;
    assert_eq!(fresh.status, McpStatus::Healthy);
    assert_eq!(rig.loader.calls().len(), 2);
}

#[tokio::test]
async fn openapi_cancelled_check_stores_nothing() {
    let rig = rig(&[], &[]);
    rig.loader.cancel_while_held(rig.cancel.clone());
    let first = rig.service.spec_health(SPEC).await;
    assert_eq!(first.status, McpStatus::Unknown);
    assert_eq!(
        first.error.as_deref(),
        Some("OpenAPI specification check was cancelled")
    );
    rig.clock.advance(Duration::from_secs(5));
    let second = rig.service.spec_health(SPEC).await;
    assert_eq!(second.status, McpStatus::Unknown);
    assert_ne!(second.checked_at, first.checked_at);
    assert_eq!(rig.loader.calls().len(), 1);
}

#[tokio::test]
async fn batch_empty_input_gives_an_empty_list() {
    let json = server_json("a", "http", "");
    let rig = rig(&[json.as_str()], &[]);
    assert!(rig.service.run_health_checks(&[]).await.is_empty());
    assert!(rig.session.calls().is_empty());
}

#[tokio::test]
async fn batch_deduplicates_and_keeps_the_input_order() {
    let b = server_json("b", "http", "");
    let a = server_json("a", "http", "");
    let rig = rig(&[b.as_str(), a.as_str()], &[]);
    let target = vec!["b".to_owned(), "a".to_owned(), "b".to_owned()];
    let rows = rig.service.run_health_checks(&target).await;
    assert_eq!(server_ids(&rows), vec!["b".to_owned(), "a".to_owned()]);
    assert_eq!(rig.session.calls().len(), 2);
}

#[tokio::test]
async fn batch_runs_ten_checks_at_a_time_and_keeps_the_order() {
    let specs: Vec<String> = (0..25)
        .map(|index| server_json(&format!("s{index}"), "http", ""))
        .collect();
    let json: Vec<&str> = specs.iter().map(String::as_str).collect();
    let rig = rig(&json, &[]);
    let target: Vec<String> = (0..25).map(|index| format!("s{index}")).collect();
    let rows = rig.service.run_health_checks(&target).await;
    assert_eq!(server_ids(&rows), target);
    assert_eq!(rig.session.calls().len(), 25);
    assert_eq!(rig.session.peak(), 10);
}

#[tokio::test]
async fn batch_health_and_teams_narrows_to_allowed_and_requested_ids() {
    let a = server_json("a", "http", "");
    let b = server_json("b", "http", "");
    let c = server_json("c", "http", "");
    let d = server_json("d", "http", "");
    let rig = rig(&[a.as_str(), b.as_str(), c.as_str()], &[d.as_str()]);
    rig.allowed
        .set_ids(vec!["c".to_owned(), "b".to_owned(), "a".to_owned()]);
    let requested = vec!["c".to_owned(), "d".to_owned(), "a".to_owned()];
    let checked = BTreeSet::from(["a".to_owned()]);
    let rows = rig
        .service
        .with_health_and_teams(None, Some(&requested), &checked)
        .await;
    assert_eq!(server_ids(&rows), vec!["c".to_owned()]);
    assert_eq!(rig.session.calls().len(), 1);
}

#[tokio::test]
async fn batch_health_and_teams_without_ids_checks_every_allowed_server() {
    let a = server_json("a", "http", "");
    let b = server_json("b", "http", "");
    let rig = rig(&[a.as_str(), b.as_str()], &[]);
    rig.allowed.set_ids(vec!["b".to_owned(), "a".to_owned()]);
    let rows = rig
        .service
        .with_health_and_teams(None, None, &BTreeSet::new())
        .await;
    assert_eq!(server_ids(&rows), vec!["b".to_owned(), "a".to_owned()]);
}

#[tokio::test]
async fn batch_an_empty_id_list_checks_everything() {
    let a = server_json("a", "http", "");
    let b = server_json("b", "http", "");
    let rig = rig(&[a.as_str(), b.as_str()], &[]);
    let empty: Vec<String> = Vec::new();
    let teams = rig
        .service
        .with_health_and_teams(None, Some(&empty), &BTreeSet::new())
        .await;
    assert_eq!(server_ids(&teams), vec!["a".to_owned(), "b".to_owned()]);
    let unfiltered = rig.service.with_health_unfiltered(Some(&empty)).await;
    assert_eq!(
        server_ids(&unfiltered),
        vec!["a".to_owned(), "b".to_owned()]
    );
}

#[tokio::test]
async fn batch_unfiltered_with_no_registered_requested_id_gives_an_empty_list() {
    let a = server_json("a", "http", "");
    let rig = rig(&[a.as_str()], &[]);
    let requested = vec!["zzz".to_owned()];
    let rows = rig.service.with_health_unfiltered(Some(&requested)).await;
    assert!(rows.is_empty());
    assert!(rig.session.calls().is_empty());
}

#[tokio::test]
async fn batch_health_and_teams_passes_the_auth_to_the_allowed_source() {
    let json = server_json("a", "http", "");
    let rig = rig(&[json.as_str()], &[]);
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(r#"{"team_id": "t1"}"#).unwrap();
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    let rows = rig
        .service
        .with_health_and_teams(Some(&auth), None, &BTreeSet::new())
        .await;
    assert_eq!(rig.allowed.calls(), vec![true]);
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn batch_unfiltered_returns_the_registry_in_order() {
    let a = server_json("a", "http", "");
    let b = server_json("b", "http", "");
    let c = server_json("c", "http", "");
    let rig = rig(&[a.as_str(), b.as_str()], &[c.as_str()]);
    let rows = rig.service.with_health_unfiltered(None).await;
    assert_eq!(
        server_ids(&rows),
        vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
    );
}

#[tokio::test]
async fn batch_unfiltered_keeps_only_registered_requested_ids() {
    let a = server_json("a", "http", "");
    let c = server_json("c", "http", "");
    let rig = rig(&[a.as_str()], &[c.as_str()]);
    let requested = vec!["c".to_owned(), "zzz".to_owned(), "a".to_owned()];
    let rows = rig.service.with_health_unfiltered(Some(&requested)).await;
    assert_eq!(server_ids(&rows), vec!["c".to_owned(), "a".to_owned()]);
}

#[tokio::test]
async fn batch_unfiltered_with_an_empty_registry_gives_an_empty_list() {
    let rig = rig(&[], &[]);
    assert!(rig.service.with_health_unfiltered(None).await.is_empty());
}

#[tokio::test]
async fn batch_unfiltered_ignores_the_allowed_source() {
    let a = server_json("a", "http", "");
    let rig = rig(&[a.as_str()], &[]);
    rig.allowed.set_ids(Vec::new());
    let rows = rig.service.with_health_unfiltered(None).await;
    assert_eq!(server_ids(&rows), vec!["a".to_owned()]);
    assert!(rig.allowed.calls().is_empty());
}

#[tokio::test]
async fn batch_all_allowed_returns_a_table_per_allowed_server_in_its_order() {
    let a = server_json("a", "http", "");
    let b = server_json("b", "http", ", \"server_name\": \"Up B\"");
    let rig = rig(&[a.as_str(), b.as_str()], &[]);
    rig.allowed.set_ids(vec!["b".to_owned(), "a".to_owned()]);
    let rows = rig.service.all_allowed(None).await;
    assert_eq!(server_ids(&rows), vec!["b".to_owned(), "a".to_owned()]);
    assert_eq!(rows[0].server_name.as_deref(), Some("Up B"));
    assert_eq!(rows[0].status, None);
    assert!(rig.session.calls().is_empty());
}

#[tokio::test]
async fn batch_all_allowed_skips_a_missing_server() {
    let a = server_json("a", "http", "");
    let rig = rig(&[a.as_str()], &[]);
    rig.allowed.set_ids(vec!["a".to_owned(), "gone".to_owned()]);
    let rows = rig.service.all_allowed(None).await;
    assert_eq!(server_ids(&rows), vec!["a".to_owned()]);
}

#[rstest]
#[case::spec_path(", \"spec_path\": \"https://up.example/spec.json\"")]
#[case::own_instructions(", \"instructions\": \"Use it\"")]
#[case::per_user_auth(", \"auth_type\": \"oauth2\"")]
#[case::missing_token(", \"auth_type\": \"api_key\"")]
#[case::empty_token(", \"auth_type\": \"api_key\", \"authentication_token\": \"\"")]
#[tokio::test]
async fn instructions_prefetch_skips_servers_that_cannot_be_probed(#[case] extra: &str) {
    let json = server_json("a", "http", extra);
    let rig = rig(&[], &[]);
    rig.service.ensure_instructions_cached(&record(&json)).await;
    assert!(rig.session.calls().is_empty());
}

#[rstest]
#[case::no_auth("")]
#[case::aws_sigv4(", \"auth_type\": \"aws_sigv4\"")]
#[case::token_present(", \"auth_type\": \"api_key\", \"authentication_token\": \"tok\"")]
#[case::blank_instructions(", \"instructions\": \"   \"")]
#[case::empty_spec_path(", \"spec_path\": \"\"")]
#[tokio::test]
async fn instructions_prefetch_probes_without_static_instructions(#[case] extra: &str) {
    let json = server_json("a", "http", extra);
    let rig = rig(&[], &[]);
    rig.session.script(Ok(Some("Use it".to_owned())));
    rig.service.ensure_instructions_cached(&record(&json)).await;
    assert_eq!(rig.session.calls().len(), 1);
}

#[tokio::test]
async fn instructions_prefetch_skips_a_per_user_env_var_server() {
    let json = server_json("a", "http", "");
    let rig = rig(&[], &[]);
    rig.env_vars.mark_per_user("a");
    rig.service.ensure_instructions_cached(&record(&json)).await;
    assert!(rig.session.calls().is_empty());
}

#[tokio::test]
async fn instructions_prefetch_caches_the_trimmed_text() {
    let rig = rig(&[], &[]);
    rig.session.script(Ok(Some("  Use it  ".to_owned())));
    let server = plain("a");
    rig.service.ensure_instructions_cached(&server).await;
    assert_eq!(rig.session.calls().len(), 1);
    rig.clock.advance(Duration::from_secs(11));
    rig.service.ensure_instructions_cached(&server).await;
    assert_eq!(rig.session.calls().len(), 1);
}

#[tokio::test]
async fn instructions_prefetch_keeps_the_cooldown_after_a_failure() {
    let rig = rig(&[], &[]);
    rig.session
        .script(Err(SessionError::Failed("upstream is down".to_owned())));
    let server = plain("a");
    rig.service.ensure_instructions_cached(&server).await;
    assert_eq!(rig.session.calls().len(), 1);
    rig.clock.advance(Duration::from_millis(9_900));
    rig.service.ensure_instructions_cached(&server).await;
    assert_eq!(rig.session.calls().len(), 1);
    rig.clock.advance(Duration::from_millis(100));
    rig.service.ensure_instructions_cached(&server).await;
    assert_eq!(rig.session.calls().len(), 2);
}

#[tokio::test]
async fn instructions_prefetch_records_the_attempt_before_the_call() {
    let rig = rig(&[], &[]);
    rig.session.script(Ok(None));
    rig.session
        .advance_on_call(Duration::from_secs(11), rig.clock.clone());
    let server = plain("a");
    rig.service.ensure_instructions_cached(&server).await;
    assert_eq!(rig.session.calls().len(), 1);
    rig.service.ensure_instructions_cached(&server).await;
    assert_eq!(rig.session.calls().len(), 2);
}

#[tokio::test]
async fn instructions_clear_empties_the_cache_and_the_cooldown() {
    let rig = rig(&[], &[]);
    rig.session.script(Ok(Some("Use it".to_owned())));
    let server = plain("a");
    rig.service.ensure_instructions_cached(&server).await;
    assert_eq!(rig.session.calls().len(), 1);
    rig.service.clear();
    rig.service.ensure_instructions_cached(&server).await;
    assert_eq!(rig.session.calls().len(), 2);
}
