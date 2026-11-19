#![cfg(test)]

use std::collections::BTreeSet;
use std::future::{Future, Ready, ready};

use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json as ContractJson;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::Builder;

use super::beta::{DANGEROUS_TOOL_USE_BETA, PER_TURN_CONTROL_BETA, beta_tokens};
use super::body::{ANTHROPIC_VERSION_FIELD, MODEL_FIELD, OutputSanitizer, apply};
use super::environment::{
    AUTHORIZATION, CONTENT_TYPE, EnvironmentRequest, JSON_CONTENT_TYPE, get_complete_url,
    validate_environment,
};
use crate::credential_sources::getters::VertexParams;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_claude_chat::output::OUTPUT_CONFIG;

const MODEL: &str = "claude-sonnet-4-6";
const TOKEN: &str = "fixed-token";
const BETA: &str = "anthropic-beta";

fn block_on<Call, Output>(future: Call) -> Output
where
    Call: Future<Output = Output>,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the test JSON parses")
}

fn object(text: &str) -> Object {
    match json(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the test JSON is an object")
        }
    }
}

fn catalog() -> ModelCatalog {
    ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("an empty catalog parses")
}

fn access() -> AccessToken {
    AccessToken {
        token: SecretString::from(TOKEN),
        project: "canonical-project".to_owned(),
    }
}

fn lookup(
    _credentials: Option<&ConfiguredCredentials>,
    _project: Option<&str>,
) -> Ready<Result<AccessToken, CredentialCacheError>> {
    ready(Ok(access()))
}

struct Validated {
    given: AnthropicHeaders,
    headers: AnthropicHeaders,
    api_base: String,
}

fn validated(
    headers: &AnthropicHeaders,
    optional_params: &Object,
    settings: &VertexSettings,
    params: VertexParams<'_>,
    api_base: Option<&str>,
) -> Validated {
    let catalog = catalog();
    let environment = block_on(validate_environment(
        &EnvironmentRequest {
            headers,
            model: MODEL,
            messages: &[],
            optional_params,
            params,
            settings,
            catalog: &catalog,
            api_base,
        },
        lookup,
    ))
    .expect("the environment validates");
    Validated {
        given: headers.clone(),
        headers: environment.headers,
        api_base: environment.api_base,
    }
}

fn header<'headers>(headers: &'headers AnthropicHeaders, name: &str) -> Option<&'headers str> {
    headers.values.get(name).map(String::as_str)
}

fn deployment() -> VertexParams<'static> {
    VertexParams {
        project: Some("deployment-project"),
        ai_project: None,
        location: None,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    }
}

#[test]
fn deployment_headers_unchanged() {
    let headers = AnthropicHeaders {
        values: IndexMap::from([
            ("x-custom".to_owned(), "keep".to_owned()),
            ("Authorization".to_owned(), "Bearer stale".to_owned()),
            (CONTENT_TYPE.to_owned(), "text/plain".to_owned()),
            (BETA.to_owned(), "old-beta, , second".to_owned()),
        ]),
    };
    let original = headers.clone();
    let outcome = validated(
        &headers,
        &Object::new(),
        &VertexSettings::default(),
        deployment(),
        None,
    );
    // the deployment's header map is the router's shared extra_headers, so the override works on a copy
    assert_eq!(outcome.given, original);
    assert_eq!(header(&outcome.headers, "x-custom"), Some("keep"));
    assert_eq!(
        header(&outcome.headers, AUTHORIZATION),
        Some("Bearer fixed-token")
    );
    assert_eq!(
        header(&outcome.headers, CONTENT_TYPE),
        Some(JSON_CONTENT_TYPE)
    );
    // the stripped empty entry survives, sorted ahead of the named ones
    assert_eq!(header(&outcome.headers, BETA), Some(",old-beta,second"));
    assert_eq!(header(&outcome.headers, "anthropic-version"), None);
    assert_eq!(
        outcome.api_base,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/deployment-project/locations/us-central1/publishers/anthropic/models/claude-sonnet-4-6:rawPredict"
    );
}

#[test]
fn a_differently_spelled_header_survives_the_overwrite() {
    let headers = AnthropicHeaders {
        values: IndexMap::from([
            ("authorization".to_owned(), "Bearer caller".to_owned()),
            ("Content-Type".to_owned(), "text/plain".to_owned()),
        ]),
    };
    let outcome = validated(
        &headers,
        &Object::new(),
        &VertexSettings::default(),
        deployment(),
        None,
    );
    assert_eq!(
        header(&outcome.headers, AUTHORIZATION),
        Some("Bearer fixed-token")
    );
    assert_eq!(
        header(&outcome.headers, "authorization"),
        Some("Bearer caller")
    );
    assert_eq!(
        header(&outcome.headers, CONTENT_TYPE),
        Some(JSON_CONTENT_TYPE)
    );
    assert_eq!(header(&outcome.headers, "Content-Type"), Some("text/plain"));
}

#[test]
fn an_absent_existing_header_gets_no_empty_beta_header() {
    let outcome = validated(
        &AnthropicHeaders::default(),
        &Object::new(),
        &VertexSettings::default(),
        deployment(),
        None,
    );
    assert_eq!(header(&outcome.headers, BETA), None);
    assert_eq!(
        header(&outcome.headers, AUTHORIZATION),
        Some("Bearer fixed-token")
    );
}

fn beta_set(optional: &str) -> BTreeSet<String> {
    beta_tokens(&AnthropicHeaders::default(), &[], &object(optional)).expect("the beta set builds")
}

fn expected(tokens: &[&str]) -> BTreeSet<String> {
    tokens.iter().map(|token| (*token).to_owned()).collect()
}

#[rstest]
#[case::compact_and_clear(
    r#"{"context_management":{"edits":[{"type":"compact_20260112"},{"type":"clear_tool_uses_20250919"}]}}"#,
    &["compact-2026-01-12", "context-management-2025-06-27"]
)]
#[case::compact_only(
    r#"{"context_management":{"edits":[{"type":"compact_20260112"}]}}"#,
    &["compact-2026-01-12"]
)]
#[case::other_only(
    r#"{"context_management":{"edits":[{"type":"clear_tool_uses_20250919"}]}}"#,
    &["context-management-2025-06-27"]
)]
#[case::compaction_is_not_compact(
    r#"{"context_management":{"edits":[{"type":"compaction"}]}}"#,
    &["context-management-2025-06-27"]
)]
#[case::type_less_edit_is_managed(
    r#"{"context_management":{"edits":[{}]}}"#,
    &["context-management-2025-06-27"]
)]
#[case::no_edits(r#"{"context_management":{"edits":[]}}"#, &[])]
#[case::no_edits_key(r#"{"context_management":{}}"#, &[])]
#[case::null_edits(r#"{"context_management":{"edits":null}}"#, &[])]
#[case::null_context_management(r#"{"context_management":null}"#, &[])]
fn compact_and_clear(#[case] optional: &str, #[case] tokens: &[&str]) {
    assert_eq!(beta_set(optional), expected(tokens));
}

#[rstest]
#[case::empty_object(r#"{"safeguards":{}}"#, &[DANGEROUS_TOOL_USE_BETA])]
#[case::true_flag(r#"{"safeguards":true}"#, &[DANGEROUS_TOOL_USE_BETA])]
#[case::false_flag(r#"{"safeguards":false}"#, &[DANGEROUS_TOOL_USE_BETA])]
#[case::absent("{}", &[])]
#[case::null_flag(r#"{"safeguards":null}"#, &[])]
fn safeguards_empty(#[case] optional: &str, #[case] tokens: &[&str]) {
    assert_eq!(beta_set(optional), expected(tokens));
}

#[rstest]
#[case::a_message_carries_it(
    r#"{"role":"user","content":[{"type":"text","text":"hi"}],"output_config":{"effort":"high"}}"#,
    &[PER_TURN_CONTROL_BETA]
)]
#[case::no_message_carries_it(r#"{"role":"user","content":"hi"}"#, &[])]
fn per_turn_output(#[case] message: &str, #[case] tokens: &[&str]) {
    let betas = beta_tokens(
        &AnthropicHeaders::default(),
        &[json(message)],
        &Object::new(),
    )
    .expect("the beta set builds");
    assert_eq!(betas, expected(tokens));
}

#[rstest]
#[case::web_search_versioned(
    r#"{"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#,
    &["web-search-2025-03-05"]
)]
#[case::web_search_bare(r#"{"tools":[{"type":"web_search"}]}"#, &["web-search-2025-03-05"])]
#[case::tool_search_regex(
    r#"{"tools":[{"type":"tool_search_tool_regex_20251119","name":"search"}]}"#,
    &["tool-search-tool-2025-10-19"]
)]
#[case::tool_search_bm25(
    r#"{"tools":[{"type":"tool_search_tool_bm25_20251119","name":"search"}]}"#,
    &["tool-search-tool-2025-10-19"]
)]
#[case::both_search_tools(
    r#"{"tools":[{"type":"web_search_20250305"},{"type":"tool_search_tool_regex_20251119"}]}"#,
    &["tool-search-tool-2025-10-19", "web-search-2025-03-05"]
)]
#[case::custom_tool_needs_no_beta(
    r#"{"tools":[{"name":"lookup","input_schema":{"type":"object"}}]}"#,
    &[]
)]
#[case::non_object_tools_are_skipped(
    r#"{"tools":[{"type":"web_search_20250305"},7]}"#,
    &["web-search-2025-03-05"]
)]
#[case::non_string_types_are_skipped(
    r#"{"tools":[{"type":7},{"type":"web_search_20250305"}]}"#,
    &["web-search-2025-03-05"]
)]
#[case::non_list_tools_are_skipped(
    r#"{"tools":{"type":"web_search_20250305"}}"#,
    &[]
)]
fn hosted_tool_betas(#[case] optional: &str, #[case] tokens: &[&str]) {
    assert_eq!(beta_set(optional), expected(tokens));
}

#[test]
fn an_existing_header_token_set_is_stripped_and_kept() {
    let headers = AnthropicHeaders {
        values: IndexMap::from([(BETA.to_owned(), " one ,,two ".to_owned())]),
    };
    let betas = beta_tokens(&headers, &[], &Object::new()).expect("the beta set builds");
    assert_eq!(betas, expected(&["", "one", "two"]));
}

#[test]
fn an_empty_existing_header_adds_nothing() {
    let headers = AnthropicHeaders {
        values: IndexMap::from([(BETA.to_owned(), String::new())]),
    };
    let betas = beta_tokens(&headers, &[], &Object::new()).expect("the beta set builds");
    assert!(betas.is_empty());
}

#[rstest]
#[case::context_management_not_object(r#"{"context_management":"compact"}"#)]
#[case::edits_not_a_list(r#"{"context_management":{"edits":{}}}"#)]
#[case::context_management_a_list(r#"{"context_management":[{"type":"compact_20260112"}]}"#)]
fn beta_shape_errors_are_400(#[case] optional: &str) {
    let error = beta_tokens(&AnthropicHeaders::default(), &[], &object(optional))
        .expect_err("the shape is rejected");
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn a_non_object_edit_is_a_400() {
    let error = beta_tokens(
        &AnthropicHeaders::default(),
        &[],
        &object(r#"{"context_management":{"edits":["clear_tool_uses_20250919"]}}"#),
    )
    .expect_err("the edit is rejected");
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        error.to_string(),
        "context_management edit 0 is not an object"
    );
}

#[test]
fn a_failed_lookup_is_a_500() {
    let catalog = catalog();
    let error = block_on(validate_environment(
        &EnvironmentRequest {
            headers: &AnthropicHeaders::default(),
            model: MODEL,
            messages: &[],
            optional_params: &Object::new(),
            params: deployment(),
            settings: &VertexSettings::default(),
            catalog: &catalog,
            api_base: None,
        },
        |_credentials, _project| ready(Err(CredentialCacheError::MissingCredentials)),
    ))
    .expect_err("the lookup fails");
    assert_eq!(error.status_code(), Some(500));
}

#[test]
fn the_configured_credentials_and_project_reach_the_lookup() {
    let credentials = ConfiguredCredentials::from_text("credentials.json".to_owned())
        .expect("the text stays a text credential");
    let catalog = catalog();
    let environment = block_on(validate_environment(
        &EnvironmentRequest {
            headers: &AnthropicHeaders::default(),
            model: MODEL,
            messages: &[],
            optional_params: &Object::new(),
            params: VertexParams {
                project: Some("deployment-project"),
                ai_project: None,
                location: None,
                ai_location: None,
                credentials: Some(&credentials),
                ai_credentials: None,
            },
            settings: &VertexSettings::default(),
            catalog: &catalog,
            api_base: None,
        },
        |given, project| {
            let forwarded = given.is_some_and(|value| value.as_text() == Some("credentials.json"));
            let hinted = project == Some("deployment-project");
            if forwarded && hinted {
                ready(Ok::<AccessToken, CredentialCacheError>(access()))
            } else {
                ready(Err(CredentialCacheError::MissingCredentials))
            }
        },
    ))
    .expect("the lookup receives the configured credentials");
    assert_eq!(
        header(&environment.headers, AUTHORIZATION),
        Some("Bearer fixed-token")
    );
}

fn url(region: &str, project: &str, endpoint: &str) -> String {
    format!(
        "https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/anthropic/models/{MODEL}:{endpoint}"
    )
}

#[rstest]
#[case::regional_default(None, None, "us-central1", "canonical-project")]
#[case::configured_location(None, Some("europe-west4"), "europe-west4", "canonical-project")]
#[case::the_deployment_project_wins(
    Some("deployment-project"),
    None,
    "us-central1",
    "deployment-project"
)]
fn the_url_uses_the_resolved_region(
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] region: &str,
    #[case] resolved: &str,
) {
    let settings = VertexSettings {
        project: None,
        location: location.map(str::to_owned),
        credentials: None,
        adc: AdcEnvironment::default(),
        // the config's DEFAULT_GOOGLE_VIDEO_DURATION_SECONDS
        default_video_duration_seconds: 8,
    };
    let outcome = validated(
        &AnthropicHeaders::default(),
        &Object::new(),
        &settings,
        VertexParams {
            project,
            ai_project: None,
            location: None,
            ai_location: None,
            credentials: None,
            ai_credentials: None,
        },
        None,
    );
    assert_eq!(outcome.api_base, url(region, resolved, "rawPredict"));
}

#[rstest]
#[case::buffered(r#"{"stream":false}"#, None, "rawPredict", None)]
#[case::streamed(r#"{"stream":true}"#, None, "streamRawPredict", None)]
#[case::streamed_through_a_proxy(
    r#"{"stream":true}"#,
    Some("https://proxy.test"),
    "streamRawPredict",
    Some("?alt=sse")
)]
fn the_stream_flag_selects_the_endpoint(
    #[case] optional: &str,
    #[case] api_base: Option<&str>,
    #[case] endpoint: &str,
    #[case] query: Option<&str>,
) {
    let outcome = validated(
        &AnthropicHeaders::default(),
        &object(optional),
        &VertexSettings::default(),
        deployment(),
        api_base,
    );
    let suffix = query.unwrap_or("");
    let expected = api_base.map_or_else(
        || {
            format!(
                "{}{suffix}",
                url("us-central1", "deployment-project", endpoint)
            )
        },
        |base| {
            format!(
                "{base}/v1/projects/deployment-project/locations/us-central1/publishers/anthropic/models/{MODEL}:{endpoint}{suffix}"
            )
        },
    );
    assert_eq!(outcome.api_base, expected);
}

#[test]
fn get_complete_url_returns_the_prepared_base() {
    let base = "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/anthropic/models/claude-sonnet-4-6:rawPredict";
    assert_eq!(get_complete_url(Some(base)).unwrap(), base);
    assert_eq!(get_complete_url(Some("")).unwrap(), "");
    let error = get_complete_url(None).expect_err("the base is required");
    assert_eq!(
        error.to_string(),
        "api_base is required. Unable to determine the correct api_base for the request."
    );
    assert_eq!(error.status_code(), Some(500));
}

#[derive(Default)]
struct Recording {
    calls: usize,
    model: Option<String>,
    saw_version: bool,
    saw_model: bool,
}

impl OutputSanitizer for Recording {
    fn sanitize<'body>(
        &'body mut self,
        body: &'body mut IndexMap<String, ContractJson>,
        model: &'body str,
    ) -> impl Future<Output = ()> + Send + 'body {
        self.calls = self.calls.saturating_add(1);
        self.model = Some(model.to_owned());
        self.saw_version = body.contains_key(ANTHROPIC_VERSION_FIELD);
        self.saw_model = body.contains_key(MODEL_FIELD);
        body.shift_remove(OUTPUT_CONFIG);
        ready(())
    }
}

struct KeepOutputConfig;

impl OutputSanitizer for KeepOutputConfig {
    fn sanitize<'body>(
        &'body mut self,
        _body: &'body mut IndexMap<String, ContractJson>,
        _model: &'body str,
    ) -> impl Future<Output = ()> + Send + 'body {
        ready(())
    }
}

#[test]
fn version_in_body() {
    let mut body = object(
        r#"{"model":"claude-sonnet-4-6","max_tokens":16,"messages":[],"output_config":{"effort":"high"}}"#,
    );
    let mut sanitizer = Recording::default();
    block_on(apply(&mut body, MODEL, &mut sanitizer)).expect("the body converts");
    assert_eq!(
        body.get(ANTHROPIC_VERSION_FIELD),
        Some(&Json::Text("vertex-2023-10-16".to_owned()))
    );
    assert!(!body.contains_key(MODEL_FIELD));
    // the policy ran on the finished body and its edit came back into the request
    assert!(!body.contains_key(OUTPUT_CONFIG));
    assert_eq!(sanitizer.calls, 1);
    assert_eq!(sanitizer.model.as_deref(), Some(MODEL));
    assert!(sanitizer.saw_version);
    assert!(!sanitizer.saw_model);
}

#[test]
fn an_absent_output_config_skips_the_sanitizer() {
    let mut body = object(r#"{"messages":[{"role":"user","content":"hi"}],"max_tokens":8}"#);
    let messages = body.get("messages").cloned();
    let mut sanitizer = Recording::default();
    block_on(apply(&mut body, MODEL, &mut sanitizer)).expect("the body converts");
    assert_eq!(sanitizer.calls, 0);
    assert_eq!(
        body.get(ANTHROPIC_VERSION_FIELD),
        Some(&Json::Text("vertex-2023-10-16".to_owned()))
    );
    assert_eq!(body.get("messages").cloned(), messages);
}

#[test]
fn the_conversion_keeps_every_json_shape() {
    let mut body = object(
        r#"{"output_config":{"effort":"high","nested":{"flag":true,"none":null,"count":7,"text":"x","list":[1,false,null,"y",{"deep":[]}]}}}"#,
    );
    block_on(apply(&mut body, MODEL, &mut KeepOutputConfig)).expect("the body converts");
    assert_eq!(
        body.get(OUTPUT_CONFIG),
        Some(&json(
            r#"{"effort":"high","nested":{"flag":true,"none":null,"count":7,"text":"x","list":[1,false,null,"y",{"deep":[]}]}}"#
        ))
    );
}

#[test]
fn a_missing_model_is_not_an_error() {
    let mut body = object(r#"{"messages":[],"output_config":{}}"#);
    let mut sanitizer = Recording::default();
    block_on(apply(&mut body, MODEL, &mut sanitizer)).expect("the body converts");
    assert_eq!(
        body.get(ANTHROPIC_VERSION_FIELD),
        Some(&Json::Text("vertex-2023-10-16".to_owned()))
    );
    assert_eq!(sanitizer.calls, 1);
}
