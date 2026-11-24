#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::settings::{JsonValue, Section, SettingsStore};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::request_context::allowlist::{
    MCP_ALLOWED_CLIENTS_SETTING, MCP_CLIENT_ID_HEADER_SETTING, McpClientAllowlist,
};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use rstest::rstest;
use serde_json::json;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::{
    allowlist_from_settings, jwt_client_field_set, proxy_error_to_http, reject_disallowed_client,
    reject_disallowed_origin, unsupported_protocol_version, version_refusal,
};

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn json_value(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the test fixture is valid JSON")
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    let built: Vec<(HeaderName, HeaderValue)> = pairs
        .iter()
        .map(|(name, value)| {
            (
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            )
        })
        .collect();
    for (name, value) in built {
        map.append(name, value);
    }
    map
}

fn store(pairs: &[(&str, JsonValue)]) -> SettingsStore {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    );
    store
}

fn auth(pairs: &[(&str, JsonValue)]) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = pairs
        .iter()
        .map(|(key, value)| {
            let decoded = serde_json::from_value(value.clone()).unwrap();
            ((*key).to_owned(), decoded)
        })
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn allowlist(
    entries: &[(&str, &str)],
    header: Option<&str>,
    jwt_field: Option<&str>,
) -> McpClientAllowlist {
    McpClientAllowlist {
        aliases_by_value: entries
            .iter()
            .map(|(value, alias)| ((*value).to_owned(), (*alias).to_owned()))
            .collect(),
        jwt_field: jwt_field.map(str::to_owned),
        header: header.map(str::to_owned),
    }
}

#[rstest]
#[case::wildcard_admits_any_origin(&["*"], &[("origin", "https://evil.test")], true)]
#[case::listed_origin_passes(&["https://ok.test"], &[("origin", "https://ok.test")], true)]
#[case::absent_origin_passes(&["https://ok.test"], &[], true)]
#[case::unknown_origin_is_refused(&["https://ok.test"], &[("origin", "https://evil.test")], false)]
#[case::second_unknown_origin_is_refused(
    &["https://ok.test"],
    &[("origin", "https://ok.test"), ("origin", "https://evil.test")],
    false
)]
#[case::empty_allowlist_refuses(&[], &[("origin", "https://ok.test")], false)]
fn an_origin_gate_matches_litellm(
    #[case] origins: &[&str],
    #[case] pairs: &[(&str, &str)],
    #[case] admitted: bool,
) {
    match reject_disallowed_origin(&names(origins), &headers(pairs)) {
        Ok(()) => assert!(admitted),
        Err(error) => {
            assert!(!admitted);
            assert_eq!(error.status, 403);
            assert_eq!(error.detail, json!("Invalid Origin header"));
            assert!(error.headers.is_empty());
        }
    }
}

#[rstest]
#[case::no_header(&[], &[], None)]
#[case::configured_value(&[("mcp-protocol-version", "2025-06-18")], &["2025-06-18"], None)]
#[case::unknown_value(
    &[("mcp-protocol-version", "1999-01-01")],
    &["2025-06-18"],
    Some("1999-01-01")
)]
#[case::blank_value_is_skipped(&[("mcp-protocol-version", "   ")], &[], None)]
#[case::surrounding_space_is_trimmed(
    &[("mcp-protocol-version", " 2025-06-18 ")],
    &["2025-06-18"],
    None
)]
#[case::first_unknown_value_wins(
    &[("mcp-protocol-version", "a"), ("mcp-protocol-version", "b")],
    &[],
    Some("a")
)]
#[case::header_name_is_case_insensitive(&[("MCP-Protocol-Version", "x")], &[], Some("x"))]
fn a_protocol_version_outside_the_list_is_refused(
    #[case] pairs: &[(&str, &str)],
    #[case] configured: &[&str],
    #[case] refused: Option<&str>,
) {
    let found = unsupported_protocol_version(&headers(pairs), &names(configured));
    assert_eq!(found.as_deref(), refused);
}

#[test]
fn a_protocol_version_reads_latin1_bytes() {
    let mut map = HeaderMap::new();
    map.append(
        "mcp-protocol-version",
        HeaderValue::from_bytes(b"v\xe9").unwrap(),
    );
    assert_eq!(
        unsupported_protocol_version(&map, &names(&[])),
        Some("v\u{e9}".to_owned())
    );
}

#[rstest]
#[case::one_version(
    &["2025-06-18"],
    "2024-01-01",
    "Unsupported MCP-Protocol-Version 2024-01-01; supported: 2025-06-18"
)]
#[case::several_versions(&["a", "b"], "c", "Unsupported MCP-Protocol-Version c; supported: a, b")]
#[case::no_supported_version(&[], "c", "Unsupported MCP-Protocol-Version c; supported: ")]
fn a_version_refusal_carries_litellms_body(
    #[case] configured: &[&str],
    #[case] value: &str,
    #[case] message: &str,
) {
    let error = version_refusal(value, &names(configured));
    assert_eq!(error.status, 400);
    assert_eq!(
        error.detail,
        json_value(&format!(
            r#"{{"jsonrpc": "2.0", "id": null, "error": {{"code": -32600, "message": "{message}"}}}}"#
        ))
    );
    assert!(error.headers.is_empty());
}

#[rstest]
#[case::parsed(Some("429"), 429)]
#[case::padded(Some(" 401 "), 401)]
#[case::plus_sign(Some("+403"), 403)]
#[case::not_a_number(Some("oops"), 500)]
#[case::decimal(Some("429.0"), 500)]
#[case::absent(None, 500)]
#[case::below_the_range(Some("99"), 500)]
#[case::first_status(Some("100"), 100)]
#[case::last_status(Some("599"), 599)]
#[case::past_the_range(Some("600"), 500)]
#[case::negative(Some("-1"), 500)]
fn a_proxy_error_status_is_the_code_or_500(#[case] code: Option<&str>, #[case] status: u16) {
    let error = proxy_error_to_http(code, "boom", &HeaderMap::new());
    assert_eq!(error.status, status);
    assert_eq!(error.detail, json!("boom"));
    assert!(error.headers.is_empty());
}

#[test]
fn a_proxy_error_keeps_its_headers() {
    let error = proxy_error_to_http(
        Some("401"),
        "Invalid proxy server token",
        &headers(&[("www-authenticate", "Bearer")]),
    );
    assert_eq!(error.status, 401);
    assert_eq!(error.detail, json!("Invalid proxy server token"));
    assert_eq!(
        error.headers,
        vec![("www-authenticate".to_owned(), "Bearer".to_owned())]
    );
}

#[test]
fn an_absent_client_list_is_not_enforced() {
    assert_eq!(allowlist_from_settings(&store(&[])), None);
}

#[test]
fn a_null_client_list_is_not_enforced() {
    assert_eq!(
        allowlist_from_settings(&store(&[(MCP_ALLOWED_CLIENTS_SETTING, JsonValue::Null)])),
        None
    );
}

#[test]
fn a_malformed_client_list_admits_nobody() {
    let settings = store(&[(MCP_ALLOWED_CLIENTS_SETTING, json!("claude"))]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert!(found.aliases_by_value.is_empty());
    assert_eq!(found.header, None);
    assert_eq!(found.jwt_field, None);
}

#[test]
fn a_malformed_client_list_still_reads_the_header() {
    let settings = store(&[
        (MCP_ALLOWED_CLIENTS_SETTING, json_value("7")),
        (MCP_CLIENT_ID_HEADER_SETTING, json!("X-Client-ID")),
    ]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert!(found.aliases_by_value.is_empty());
    assert_eq!(found.header.as_deref(), Some("x-client-id"));
}

#[test]
fn a_client_list_pairs_each_value_with_its_alias() {
    let settings = store(&[(
        MCP_ALLOWED_CLIENTS_SETTING,
        json_value(r#"[{"alias": "claude-code", "value": "client-1"}]"#),
    )]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert_eq!(
        found.aliases_by_value,
        BTreeMap::from([("client-1".to_owned(), "claude-code".to_owned())])
    );
    assert_eq!(found.header, None);
    assert_eq!(found.jwt_field, None);
}

#[test]
fn a_client_entry_without_an_alias_admits_nobody() {
    let settings = store(&[(
        MCP_ALLOWED_CLIENTS_SETTING,
        json_value(r#"[{"alias": "claude-code", "value": "client-1"}, {"value": "client-2"}]"#),
    )]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert!(found.aliases_by_value.is_empty());
}

#[test]
fn a_client_entry_with_a_blank_value_admits_nobody() {
    let settings = store(&[(
        MCP_ALLOWED_CLIENTS_SETTING,
        json_value(r#"[{"alias": "claude-code", "value": ""}]"#),
    )]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert!(found.aliases_by_value.is_empty());
}

#[test]
fn a_client_id_header_is_lowercased() {
    let settings = store(&[
        (MCP_ALLOWED_CLIENTS_SETTING, json!([])),
        (MCP_CLIENT_ID_HEADER_SETTING, json!("X-Client-ID")),
    ]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert_eq!(found.header.as_deref(), Some("x-client-id"));
}

#[test]
fn a_blank_client_id_header_is_no_header() {
    let settings = store(&[
        (MCP_ALLOWED_CLIENTS_SETTING, json!([])),
        (MCP_CLIENT_ID_HEADER_SETTING, json!("")),
    ]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert_eq!(found.header, None);
}

#[test]
fn a_header_setting_of_the_wrong_type_is_ignored() {
    let settings = store(&[
        (MCP_ALLOWED_CLIENTS_SETTING, json!([])),
        (MCP_CLIENT_ID_HEADER_SETTING, json!(["x-client-id"])),
    ]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert_eq!(found.header, None);
}

#[test]
fn a_jwt_field_comes_from_litellm_jwtauth() {
    let settings = store(&[
        (MCP_ALLOWED_CLIENTS_SETTING, json!([])),
        (
            "litellm_jwtauth",
            json_value(r#"{"mcp_client_id_jwt_field": "azp"}"#),
        ),
    ]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert_eq!(found.jwt_field.as_deref(), Some("azp"));
}

#[test]
fn a_jwt_auth_section_of_the_wrong_type_is_ignored() {
    let settings = store(&[
        (MCP_ALLOWED_CLIENTS_SETTING, json!([])),
        ("litellm_jwtauth", json!("azp")),
    ]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert_eq!(found.jwt_field, None);
}

#[test]
fn a_blank_jwt_field_is_no_field() {
    let settings = store(&[
        (MCP_ALLOWED_CLIENTS_SETTING, json!([])),
        (
            "litellm_jwtauth",
            json_value(r#"{"mcp_client_id_jwt_field": ""}"#),
        ),
    ]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert_eq!(found.jwt_field, None);
}

#[test]
fn a_null_jwt_field_is_no_field() {
    let settings = store(&[
        (MCP_ALLOWED_CLIENTS_SETTING, json!([])),
        (
            "litellm_jwtauth",
            json_value(r#"{"mcp_client_id_jwt_field": null}"#),
        ),
    ]);
    let found = allowlist_from_settings(&settings).unwrap();
    assert_eq!(found.jwt_field, None);
}

#[rstest]
#[case::absent(&[], false)]
#[case::null_list(&[(MCP_ALLOWED_CLIENTS_SETTING, JsonValue::Null)], false)]
#[case::list_without_a_field(&[(MCP_ALLOWED_CLIENTS_SETTING, json!([]))], false)]
#[case::field_outside_litellm_jwtauth(
    &[
        (MCP_ALLOWED_CLIENTS_SETTING, json!([])),
        ("litellm_settings", json_value(r#"{"mcp_client_id_jwt_field": "azp"}"#)),
    ],
    false
)]
#[case::list_with_a_field(
    &[
        (MCP_ALLOWED_CLIENTS_SETTING, json!([])),
        ("litellm_jwtauth", json_value(r#"{"mcp_client_id_jwt_field": "azp"}"#)),
    ],
    true
)]
fn jwt_client_field_set_answers_from_the_allowlist(
    #[case] pairs: &[(&str, JsonValue)],
    #[case] expected: bool,
) {
    assert_eq!(jwt_client_field_set(&store(pairs)), expected);
}

#[test]
fn an_unenforced_allowlist_admits_every_client() {
    reject_disallowed_client(None, &headers(&[]), None).unwrap();
}

#[test]
fn a_listed_client_passes() {
    let allowed = allowlist(&[("client-1", "claude-code")], Some("x-client-id"), None);
    reject_disallowed_client(
        Some(&allowed),
        &headers(&[("x-client-id", "client-1")]),
        None,
    )
    .unwrap();
}

#[test]
fn an_unlisted_client_is_forbidden_with_litellms_body() {
    let allowed = allowlist(&[("client-1", "claude-code")], Some("x-client-id"), None);
    let error = reject_disallowed_client(
        Some(&allowed),
        &headers(&[("x-client-id", "client-2")]),
        None,
    )
    .unwrap_err();
    assert_eq!(error.status, 403);
    assert_eq!(
        error.detail,
        json_value(
            r#"{"error": "Forbidden", "details": "MCP client 'client-2' (from header 'x-client-id') is not listed in this gateway's mcp_allowed_clients."}"#
        )
    );
    assert!(error.headers.is_empty());
}

#[test]
fn a_missing_client_header_is_forbidden() {
    let allowed = allowlist(&[("client-1", "claude-code")], Some("x-client-id"), None);
    let error = reject_disallowed_client(Some(&allowed), &headers(&[]), None).unwrap_err();
    assert_eq!(error.status, 403);
    assert_eq!(
        error.detail,
        json_value(
            r#"{"error": "Forbidden", "details": "The request has no 'x-client-id' header naming the client application. This gateway only admits client applications listed in mcp_allowed_clients."}"#
        )
    );
}

#[test]
fn a_dashboard_session_is_not_a_client_application() {
    let allowed = allowlist(&[("client-1", "claude-code")], Some("x-client-id"), None);
    let session = auth(&[
        ("team_id", json!("litellm-dashboard")),
        ("user_id", json!("user-1")),
    ]);
    reject_disallowed_client(Some(&allowed), &headers(&[]), Some(&session)).unwrap();
}

#[test]
fn a_jwt_claim_names_the_client_when_the_allowlist_has_a_field() {
    let allowed = allowlist(
        &[("client-1", "claude-code")],
        Some("x-client-id"),
        Some("azp"),
    );
    let caller = auth(&[("jwt_claims", json_value(r#"{"azp": "client-1"}"#))]);
    reject_disallowed_client(Some(&allowed), &headers(&[]), Some(&caller)).unwrap();
}

#[test]
fn a_jwt_claim_outranks_the_client_header() {
    let allowed = allowlist(
        &[("client-1", "claude-code")],
        Some("x-client-id"),
        Some("azp"),
    );
    let caller = auth(&[("jwt_claims", json_value(r#"{"azp": "client-9"}"#))]);
    let error = reject_disallowed_client(
        Some(&allowed),
        &headers(&[("x-client-id", "client-1")]),
        Some(&caller),
    )
    .unwrap_err();
    assert_eq!(error.status, 403);
}

#[test]
fn an_auth_without_claims_falls_back_to_the_header() {
    let allowed = allowlist(
        &[("client-1", "claude-code")],
        Some("x-client-id"),
        Some("azp"),
    );
    let caller = auth(&[("user_id", json!("user-1"))]);
    reject_disallowed_client(
        Some(&allowed),
        &headers(&[("x-client-id", "client-1")]),
        Some(&caller),
    )
    .unwrap();
}

#[test]
fn a_rejection_logs_litellms_warning() {
    let (logs, guard) = capture_logs();
    let allowed = allowlist(&[("client-1", "claude-code")], Some("x-client-id"), None);
    let rejection = reject_disallowed_client(
        Some(&allowed),
        &headers(&[("x-client-id", "client-2")]),
        None,
    );
    drop(guard);
    let error = rejection.unwrap_err();
    assert_eq!(error.status, 403);
    assert!(logs.contents().contains(
        "Rejected MCP request from a disallowed client application: MCP client 'client-2'"
    ));
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
