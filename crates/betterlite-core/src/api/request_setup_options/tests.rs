#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use uuid::Uuid;

use super::callbacks::CallbackSettings;
use super::destinations::disabled_backends;
use super::error::RequestSetupOptionsError;
use super::{
    CallbackSource, CallbackVarsDecryption, RandomSessionIds, RequestSetupOptions,
    RequestSetupOptionsInput, RequestSetupOptionsResult, RequestSetupSettings, SessionIdSource,
    SetupDiagnostic,
};
use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::metadata::MetadataField;
use crate::api::request_principal_metadata::{Metadata, PrincipalFacts};

const CHAT_ROUTE: &str = "/v1/chat/completions";
const LANGFUSE_KEYS: &str =
    r#""callback_vars": {"langfuse_public_key": "pk", "langfuse_secret_key": "sk"}"#;

struct FixedIds;

impl SessionIdSource for FixedIds {
    fn session_id(&self) -> String {
        "generated-id".to_owned()
    }
}

// stands in for the salt-key cipher: a value marked sealed: comes back with the mark removed
struct Unseal;

impl CallbackVarsDecryption for Unseal {
    fn decrypt(&self, metadata: &Metadata) -> Metadata {
        let text = serde_json::to_string(metadata).unwrap();
        serde_json::from_str(&text.replace("sealed:", "")).unwrap()
    }
}

type Service = RequestSetupOptions<FixedIds, Unseal, BTreeMap<String, String>>;

fn settings() -> RequestSetupSettings {
    RequestSetupSettings {
        allow_dynamic_callback_disabling: true,
        missing_session_id: None,
        drop_params_configured: false,
        otel_v2_enabled: true,
        default_team_settings: Vec::new(),
        allowed_hosts: Vec::new(),
    }
}

fn service(settings: RequestSetupSettings) -> Service {
    RequestSetupOptions {
        settings,
        session_ids: FixedIds,
        decryption: Unseal,
        environment: BTreeMap::new(),
    }
}

fn principal(text: &str) -> PrincipalFacts {
    serde_json::from_str(text).unwrap()
}

fn input(data: &str, route: &str, principal_text: &str) -> RequestSetupOptionsInput {
    RequestSetupOptionsInput {
        data: serde_json::from_str::<RequestData>(data).unwrap(),
        route: route.to_owned(),
        query: Vec::new(),
        user_agent: String::new(),
        metadata_variable_name: MetadataField::Metadata,
        principal: principal(principal_text),
    }
}

fn prepared(
    settings: RequestSetupSettings,
    request: RequestSetupOptionsInput,
) -> Result<RequestSetupOptionsResult, RequestSetupOptionsError> {
    service(settings).prepare(request)
}

type Lists = (Vec<String>, Vec<String>, Vec<String>, Vec<(String, String)>);

fn lists(callbacks: &CallbackSettings) -> Lists {
    (
        callbacks.success_callback.clone(),
        callbacks.failure_callback.clone(),
        callbacks.callbacks.clone(),
        callbacks
            .callback_vars
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect(),
    )
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn vars(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::key_logging_wins(
    r#"{"metadata": {"logging": [{"callback_name": "langfuse", "callback_vars": {"langfuse_host": "sealed:https://key"}}]}, "team_metadata": {"logging": [{"callback_name": "team", "callback_vars": {}}]}}"#,
    Some((names(&["langfuse"]), names(&["langfuse"]), names(&["langfuse"]), vars(&[("langfuse_host", "https://key")]))),
    Vec::new()
)]
#[case::key_empty_list_blocks_team(
    r#"{"metadata": {"logging": []}, "team_metadata": {"logging": [{"callback_name": "team", "callback_vars": {}}]}, "team_id": "t1"}"#,
    None,
    Vec::new()
)]
#[case::key_null_falls_to_team(
    r#"{"metadata": {"logging": null}, "team_metadata": {"logging": [{"callback_name": "team", "callback_type": "failure", "callback_vars": {}}]}}"#,
    Some((Vec::new(), names(&["team"]), Vec::new(), Vec::new())),
    Vec::new()
)]
#[case::team_logging_beats_callback_settings(
    r#"{"team_metadata": {"logging": [{"callback_name": "team", "callback_type": "success", "callback_vars": {}}], "callback_settings": {"success_callback": ["old"]}}}"#,
    Some((names(&["team"]), Vec::new(), Vec::new(), Vec::new())),
    Vec::new()
)]
#[case::deprecated_callback_settings(
    r#"{"team_metadata": {"callback_settings": {"success_callback": ["langfuse"], "failure_callback": null, "callback_vars": {"langfuse_public_key": "sealed:pk"}}}, "team_id": "t1"}"#,
    Some((names(&["langfuse"]), Vec::new(), Vec::new(), vars(&[("langfuse_public_key", "pk")]))),
    Vec::new()
)]
#[case::deprecated_null_settings(
    r#"{"team_metadata": {"callback_settings": null}}"#,
    Some((Vec::new(), Vec::new(), Vec::new(), Vec::new())),
    Vec::new()
)]
#[case::config_default_team_settings(
    r#"{"team_id": "t1"}"#,
    Some((names(&["langsmith"]), Vec::new(), Vec::new(), vars(&[("langsmith_api_key", "ls")]))),
    Vec::new()
)]
#[case::team_not_in_config(r#"{"team_id": "t2"}"#, None, Vec::new())]
#[case::no_source(r#"{"metadata": {"tags": ["a"]}}"#, None, Vec::new())]
#[case::invalid_entries_skipped(
    r#"{"metadata": {"logging": [
        {"callback_name": "bad", "callback_vars": {"unknown_var": "x"}},
        {"callback_name": "env", "callback_vars": {"langfuse_host": "os.environ/HOST"}},
        {"callback_name": "langfuse", "callback_vars": {"langfuse_environment": "Prod"}},
        {"callback_name": "langfuse_otel", "callback_vars": {"langfuse_span_scope": "most"}},
        {"callback_name": "missing_vars"},
        "not-an-object"
    ]}}"#,
    None,
    vec![SetupDiagnostic::InvalidCallbackMetadata { source: CallbackSource::Key }; 6]
)]
#[case::team_list_not_a_list(
    r#"{"team_metadata": {"logging": {"callback_name": "langfuse"}}}"#,
    None,
    vec![SetupDiagnostic::InvalidCallbackMetadata { source: CallbackSource::Team }]
)]
#[case::entries_merge_in_order(
    r#"{"metadata": {"logging": [
        {"callback_name": "langfuse", "callback_type": "success", "callback_vars": {"langfuse_public_key": "a", "newrelic_api_key": "nr"}},
        {"callback_name": "langfuse", "callback_type": null, "callback_vars": {"langfuse_public_key": "b", "signoz_ingestion_key": "sz"}},
        {"callback_name": "newrelic", "callback_type": "failure", "callback_vars": {"newrelic_api_key": "own", "turn_off_message_logging": true}},
        {"callback_name": "signoz", "callback_type": "success_and_failure", "callback_vars": {"signoz_ingestion_key": "own", "dd_agent_port": 8126, "langfuse_host": null}},
        {"callback_name": "x", "callback_type": "", "callback_vars": {}}
    ]}}"#,
    Some((
        names(&["langfuse", "signoz"]),
        names(&["langfuse", "newrelic", "signoz"]),
        names(&["langfuse", "signoz"]),
        vars(&[
            ("langfuse_public_key", "b"),
            ("newrelic_api_key", "own"),
            ("turn_off_message_logging", "True"),
            ("signoz_ingestion_key", "own"),
            ("dd_agent_port", "8126"),
            ("langfuse_host", "None"),
        ])
    )),
    vec![SetupDiagnostic::InvalidCallbackMetadata { source: CallbackSource::Key }]
)]
fn metadata_slot_selection(
    #[case] principal_text: &str,
    #[case] expected: Option<Lists>,
    #[case] expected_diagnostics: Vec<SetupDiagnostic>,
) {
    let mut configured = settings();
    configured.default_team_settings = serde_json::from_str(
        r#"[{"team_id": "t1", "success_callback": ["langsmith"], "langsmith_api_key": "ls"}]"#,
    )
    .unwrap();
    let result = prepared(configured, input("{}", CHAT_ROUTE, principal_text)).unwrap();
    assert_eq!(result.callback_settings.as_ref().map(lists), expected);
    assert_eq!(result.diagnostics, expected_diagnostics);
}

#[rstest]
#[case::stored_variable_unknown(
    r#"{"team_metadata": {"callback_settings": {"callback_vars": {"not_a_param": "x"}}}}"#
)]
#[case::stored_shape_wrong(r#"{"team_metadata": {"callback_settings": ["langfuse"]}}"#)]
#[case::stored_value_not_text(
    r#"{"team_metadata": {"callback_settings": {"callback_vars": {"langfuse_host": 5}}}}"#
)]
#[case::config_variable_unknown(r#"{"team_id": "bad"}"#)]
fn stored_callback_settings_fail_the_request(#[case] principal_text: &str) {
    let mut configured = settings();
    configured.default_team_settings =
        serde_json::from_str(r#"[{"team_id": "bad", "callback_vars": {"not_a_param": "x"}}]"#)
            .unwrap();
    let error = prepared(configured, input("{}", CHAT_ROUTE, principal_text)).unwrap_err();
    let facts = error.facts();
    assert_eq!(
        (facts.status, facts.error_type, facts.param, facts.code),
        (500, "internal_server_error", None, "500")
    );
}

#[rstest]
#[case::header_wins(true, r#"{"litellm_disabled_callbacks": ["Key"]}"#, Some(" Langfuse , ARIZE"), &["arize", "langfuse"])]
#[case::empty_header_names_nothing_useful(true, r#"{"litellm_disabled_callbacks": ["key"]}"#, Some(""), &[""])]
#[case::key_list_lowercased(true, r#"{"litellm_disabled_callbacks": ["LangFuse", 5, null, " spaced "]}"#, None, &["langfuse", " spaced "])]
#[case::key_value_not_a_list(true, r#"{"litellm_disabled_callbacks": "langfuse"}"#, None, &[])]
#[case::switched_off(false, r#"{"litellm_disabled_callbacks": ["langfuse"]}"#, Some("arize"), &[])]
#[case::nothing_disabled(true, "{}", None, &[])]
fn dynamic_callback_disabling(
    #[case] allowed: bool,
    #[case] key_metadata: &str,
    #[case] header: Option<&str>,
    #[case] expected: &[&str],
) {
    let metadata: Metadata = serde_json::from_str(key_metadata).unwrap();
    let mut headers = IndexMap::from([("x-other".to_owned(), "langfuse".to_owned())]);
    if let Some(value) = header {
        headers.insert("X-LiteLLM-Disable-Callbacks".to_owned(), value.to_owned());
    }
    let expected_names: BTreeSet<String> = expected.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(
        disabled_backends(allowed, &metadata, &headers),
        expected_names
    );
}

fn langfuse_entry(callback_type: &str) -> String {
    format!(
        r#"{{"callback_name": "langfuse_otel", "callback_type": "{callback_type}", {LANGFUSE_KEYS}}}"#
    )
}

#[rstest]
#[case::key_destination(true, true, format!(r#"{{"metadata": {{"logging": [{}], "otel_service_name": "  "}}, "team_metadata": {{"otel_service_name_override": " team-svc "}}}}"#, langfuse_entry("success")), None, Some(Some("team-svc")))]
#[case::team_destination(true, true, format!(r#"{{"metadata": {{"otel_service_name": "key-svc"}}, "team_metadata": {{"logging": [{}]}}}}"#, langfuse_entry("success_and_failure")), None, Some(Some("key-svc")))]
#[case::key_empty_list_blocks_team(true, true, format!(r#"{{"metadata": {{"logging": []}}, "team_metadata": {{"logging": [{}]}}}}"#, langfuse_entry("success")), None, None)]
#[case::otel_v2_off(false, true, format!(r#"{{"metadata": {{"logging": [{}]}}}}"#, langfuse_entry("success")), None, None)]
#[case::header_disables(true, true, format!(r#"{{"metadata": {{"logging": [{}]}}}}"#, langfuse_entry("success")), Some("LANGFUSE_OTEL"), None)]
#[case::key_list_disables(true, true, format!(r#"{{"metadata": {{"logging": [{}], "litellm_disabled_callbacks": ["langfuse_otel"]}}}}"#, langfuse_entry("success")), None, None)]
#[case::disabling_switched_off(true, false, format!(r#"{{"metadata": {{"logging": [{}]}}}}"#, langfuse_entry("success")), Some("langfuse_otel"), Some(None))]
#[case::failure_only(true, true, format!(r#"{{"metadata": {{"logging": [{}]}}}}"#, langfuse_entry("failure")), None, None)]
#[case::failure_entry_keys_merge(true, true, r#"{"metadata": {"logging": [{"callback_name": "langfuse_otel", "callback_type": "success", "callback_vars": {"langfuse_public_key": "pk"}}, {"callback_name": "langfuse_otel", "callback_type": "failure", "callback_vars": {"langfuse_secret_key": "sk"}}]}}"#.to_owned(), None, Some(None))]
#[case::invalid_bool_empties_params(true, true, r#"{"metadata": {"logging": [{"callback_name": "langfuse_otel", "callback_vars": {"langfuse_public_key": "pk", "langfuse_secret_key": "sk", "turn_off_message_logging": "maybe"}}]}}"#.to_owned(), None, None)]
fn tenant_destinations(
    #[case] otel_v2_enabled: bool,
    #[case] allow_dynamic_callback_disabling: bool,
    #[case] principal_text: String,
    #[case] header: Option<&str>,
    #[case] expected_service: Option<Option<&str>>,
) {
    let mut configured = settings();
    configured.otel_v2_enabled = otel_v2_enabled;
    configured.allow_dynamic_callback_disabling = allow_dynamic_callback_disabling;
    let headers: IndexMap<String, String> = header
        .map(|value| ("x-litellm-disable-callbacks".to_owned(), value.to_owned()))
        .into_iter()
        .collect();
    let resolved = service(configured).tenant_destinations(&principal(&principal_text), &headers);
    let found: Vec<_> = resolved.destinations.iter().map(AsRef::as_ref).collect();
    match (found.as_slice(), expected_service) {
        ([], None) => {}
        ([destination], Some(service_name)) => {
            assert_eq!(
                destination.endpoint,
                "https://us.cloud.langfuse.com/api/public/otel"
            );
            assert_eq!(destination.callback_name.as_deref(), Some("langfuse_otel"));
            assert!(destination.headers.contains_key("Authorization"));
            assert_eq!(destination.span_scope, SpanScope::Full);
            assert_eq!(
                destination
                    .resource_attributes
                    .get("service.name")
                    .map(String::as_str),
                service_name
            );
        }
        (other, want) => panic!("{want:?} expected, got {other:?}"),
    }
    assert!(resolved.diagnostics.is_empty());
}

#[rstest]
#[case::prompt_version_integer(r#""langfuse_prompt_version": " 3 ""#, true)]
#[case::prompt_version_text(r#""langfuse_prompt_version": "three""#, false)]
#[case::sampling_rate_decimal(r#""langsmith_sampling_rate": "0.5""#, true)]
#[case::sampling_rate_scientific(r#""arize_error_sampling_rate": "5e-1""#, true)]
#[case::sampling_rate_text(r#""arize_success_sampling_rate": "half""#, false)]
#[case::bool_word(r#""turn_off_message_logging": "Yes""#, true)]
#[case::disabled_list_never_fits(r#""litellm_disabled_callbacks": "arize""#, false)]
fn tenant_params_follow_the_declared_types(#[case] extra_var: &str, #[case] kept: bool) {
    let principal_text = format!(
        r#"{{"metadata": {{"logging": [{{"callback_name": "langfuse_otel", "callback_vars": {{"langfuse_public_key": "pk", "langfuse_secret_key": "sk", {extra_var}}}}}]}}}}"#
    );
    let resolved =
        service(settings()).tenant_destinations(&principal(&principal_text), &IndexMap::new());
    assert_eq!(resolved.destinations.len(), usize::from(kept));
}

#[test]
fn tenant_destinations_report_invalid_entries() {
    let principal_text = format!(
        r#"{{"metadata": {{"logging": [{{"callback_name": "langfuse_otel", "callback_vars": {{"bogus": "x"}}}}, {}]}}}}"#,
        langfuse_entry("success")
    );
    let resolved =
        service(settings()).tenant_destinations(&principal(&principal_text), &IndexMap::new());
    assert_eq!(resolved.destinations.len(), 1);
    assert_eq!(
        resolved.diagnostics,
        vec![SetupDiagnostic::InvalidCallbackMetadata {
            source: CallbackSource::OtelDestination
        }]
    );
}

#[rstest]
#[case::generate_new_id(
    Some(MissingSessionIdPolicy::Generate),
    CHAT_ROUTE,
    r#"{"metadata": {}}"#,
    Ok(r#"{"metadata": {"session_id": "generated-id", "litellm_session_id_generated": true}, "litellm_session_id": "generated-id", "litellm_trace_id": "generated-id"}"#)
)]
#[case::generate_from_body_trace(
    Some(MissingSessionIdPolicy::Generate),
    CHAT_ROUTE,
    r#"{"litellm_trace_id": "trace-1", "litellm_session_id": null, "metadata": {"trace_id": "meta-trace", "session_id": null}}"#,
    Ok(r#"{"litellm_trace_id": "trace-1", "litellm_session_id": "trace-1", "metadata": {"trace_id": "meta-trace", "session_id": "trace-1", "litellm_session_id_generated": true}}"#)
)]
#[case::generate_from_metadata_trace(
    Some(MissingSessionIdPolicy::Generate),
    CHAT_ROUTE,
    r#"{"litellm_trace_id": null, "metadata": {"trace_id": "meta-trace"}}"#,
    Ok(r#"{"litellm_trace_id": "meta-trace", "metadata": {"trace_id": "meta-trace", "session_id": "meta-trace", "litellm_session_id_generated": true}, "litellm_session_id": "meta-trace"}"#)
)]
#[case::reject_without_session(Some(MissingSessionIdPolicy::Reject), CHAT_ROUTE, r#"{"metadata": {}}"#, Err(()))]
#[case::reject_with_body_session(
    Some(MissingSessionIdPolicy::Reject),
    CHAT_ROUTE,
    r#"{"litellm_session_id": "s1", "metadata": {}}"#,
    Ok(r#"{"litellm_session_id": "s1", "metadata": {}}"#)
)]
#[case::reject_with_metadata_session(
    Some(MissingSessionIdPolicy::Reject),
    CHAT_ROUTE,
    r#"{"metadata": {"session_id": "s1"}}"#,
    Ok(r#"{"metadata": {"session_id": "s1"}}"#)
)]
#[case::reject_off_inference_route(
    Some(MissingSessionIdPolicy::Reject),
    "/key/generate",
    r#"{"metadata": {}}"#,
    Ok(r#"{"metadata": {}}"#)
)]
#[case::reject_on_mcp_route(
    Some(MissingSessionIdPolicy::Reject),
    "/mcp",
    r#"{"metadata": {}}"#,
    Ok(r#"{"metadata": {}}"#)
)]
#[case::reject_without_metadata_object(
    Some(MissingSessionIdPolicy::Reject),
    CHAT_ROUTE,
    r#"{"metadata": "text"}"#,
    Ok(r#"{"metadata": "text"}"#)
)]
#[case::omit_left_to_the_safety_stage(
    Some(MissingSessionIdPolicy::Omit),
    CHAT_ROUTE,
    r#"{"metadata": {}}"#,
    Ok(r#"{"metadata": {}}"#)
)]
#[case::unknown_policy_warns(
    Some(MissingSessionIdPolicy::Unrecognized("pause".to_owned())),
    CHAT_ROUTE,
    r#"{"metadata": {}}"#,
    Ok(r#"{"metadata": {}}"#)
)]
#[case::no_policy(None, CHAT_ROUTE, r#"{"metadata": {}}"#, Ok(r#"{"metadata": {}}"#))]
fn missing_session_policy(
    #[case] policy: Option<MissingSessionIdPolicy>,
    #[case] route: &str,
    #[case] data: &str,
    #[case] expected: Result<&str, ()>,
) {
    let mut configured = settings();
    configured.missing_session_id = policy.clone();
    let outcome = prepared(configured, input(data, route, "{}"));
    match (outcome, expected) {
        (Ok(result), Ok(expected_data)) => {
            assert_eq!(
                result.data,
                serde_json::from_str::<RequestData>(expected_data).unwrap()
            );
            let warned = matches!(policy, Some(MissingSessionIdPolicy::Unrecognized(_)));
            let expected_diagnostics: Vec<SetupDiagnostic> = warned
                .then(|| SetupDiagnostic::UnknownSessionPolicy {
                    policy: "pause".to_owned(),
                })
                .into_iter()
                .collect();
            assert_eq!(result.diagnostics, expected_diagnostics);
        }
        (Err(error), Err(())) => {
            let facts = error.facts();
            assert_eq!(
                (facts.status, facts.error_type, facts.param, facts.code),
                (400, "bad_request_error", Some("session_id"), "400")
            );
        }
        (other, want) => panic!("{want:?} expected, got {other:?}"),
    }
}

#[test]
fn missing_session_policy_uses_the_selected_slot() {
    let mut configured = settings();
    configured.missing_session_id = Some(MissingSessionIdPolicy::Generate);
    let mut request = input(
        r#"{"metadata": {"session_id": "requester"}, "litellm_metadata": {}}"#,
        CHAT_ROUTE,
        "{}",
    );
    request.metadata_variable_name = MetadataField::LitellmMetadata;
    let result = prepared(configured, request).unwrap();
    let expected: RequestData = serde_json::from_str(
        r#"{"metadata": {"session_id": "requester"}, "litellm_metadata": {"session_id": "generated-id", "litellm_session_id_generated": true}, "litellm_session_id": "generated-id", "litellm_trace_id": "generated-id"}"#,
    )
    .unwrap();
    assert_eq!(result.data, expected);
}

#[rstest]
#[case::claude_cli("claude-cli/2.1.0", "{}", false, Some(Json::Bool(true)))]
#[case::claude_code("claude-code/1.0", "{}", false, Some(Json::Bool(true)))]
#[case::codex_any_case("CODEX_cli_rs/0.1", "{}", false, Some(Json::Bool(true)))]
#[case::codex_space("codex exec", "{}", false, Some(Json::Bool(true)))]
#[case::codex_slash("codex/1", "{}", false, Some(Json::Bool(true)))]
#[case::codex_dash("codex-tui", "{}", false, Some(Json::Bool(true)))]
#[case::codex_without_separator("codexcli", "{}", false, None)]
#[case::codex_alone("codex", "{}", false, None)]
#[case::other_client("curl/8.0", "{}", false, None)]
#[case::caller_set_false(
    "claude-cli/2.1.0",
    r#"{"drop_params": false}"#,
    false,
    Some(Json::Bool(false))
)]
#[case::config_decides("claude-cli/2.1.0", "{}", true, None)]
fn agentic_cli_drop_params(
    #[case] user_agent: &str,
    #[case] data: &str,
    #[case] drop_params_configured: bool,
    #[case] expected: Option<Json>,
) {
    let mut configured = settings();
    configured.drop_params_configured = drop_params_configured;
    let mut request = input(data, CHAT_ROUTE, "{}");
    request.user_agent = user_agent.to_owned();
    let result = prepared(configured, request).unwrap();
    assert_eq!(result.data.get("drop_params"), expected.as_ref());
}

#[rstest]
#[case::absent(&[("x", "1")], None)]
#[case::single(&[("api-version", "2024-06-01")], Some("2024-06-01"))]
#[case::last_repeat_wins(&[("api-version", "2024-01-01"), ("api-version", "2024-06-01")], Some("2024-06-01"))]
fn api_version_from_query(#[case] query: &[(&str, &str)], #[case] expected: Option<&str>) {
    let mut request = input("{}", CHAT_ROUTE, "{}");
    request.query = query
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let result = prepared(settings(), request).unwrap();
    assert_eq!(
        result.data.get("api_version"),
        expected
            .map(|version| Json::Text(version.to_owned()))
            .as_ref()
    );
}

#[rstest]
#[case::key(CallbackSource::Key, "key-level")]
#[case::team(CallbackSource::Team, "team-level")]
#[case::destination(CallbackSource::OtelDestination, "otel-destination")]
fn callback_sources_name_their_level(#[case] source: CallbackSource, #[case] expected: &str) {
    assert_eq!(source.as_str(), expected);
}

#[test]
fn random_session_ids_are_uuids() {
    let first = RandomSessionIds.session_id();
    assert_eq!(Uuid::parse_str(&first).unwrap().to_string(), first);
    assert_ne!(first, RandomSessionIds.session_id());
}
