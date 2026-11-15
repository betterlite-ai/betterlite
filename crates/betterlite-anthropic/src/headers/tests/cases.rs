use std::collections::{BTreeMap, BTreeSet};

use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::credentials::{
    ANTHROPIC_VERSION, AnthropicSettings, BROWSER_ACCESS_HEADER, OAUTH_BETA_HEADER,
    OAUTH_BETA_HEADER_NAME,
};
use crate::feature_flags::beta::{
    EFFORT_BETA, FILE_CODE_EXECUTION_BETA, FILES_API_BETA, MCP_CLIENT_BETA, OUTPUT_CONFIG_BETA,
    THINKING_DISPLAY_BETA, TOOL_CHANGES_BETA,
};
use crate::headers::catalog::{ModelListResponse, ModelRecord, anthropic_model_list_response};
use crate::headers::environment::{
    EnvironmentRequest, HeaderLiteLlmParams, HeaderParams, validate_environment,
};
use crate::headers::request::{
    ACCEPT_HEADER, ANTHROPIC_VERSION_HEADER, AUTHORIZATION_HEADER, CODE_EXECUTION_BETA,
    CONTENT_TYPE_HEADER, HeaderFeature, HeaderFeatures, HeaderRequest, JSON_CONTENT_TYPE,
    SKILLS_BETA, TOOL_SEARCH_BETA, WEB_SEARCH_BETA, get_anthropic_headers,
};
use crate::headers::tests::{fixture, projection};
use crate::headers::{AnthropicHeaders, HeadersError};

const X_API_KEY: &str = "x-api-key";
const MODEL: &str = "claude-sonnet-4";

#[rstest]
#[case::default_native_version(0)]
#[case::api_key_and_bearer(1)]
#[case::oauth_beta_merge(2)]
#[case::derived_beta_union(3)]
#[case::vertex_web_search_beta(4)]
#[case::user_beta_preserved(5)]
#[case::response_header_prefixes(6)]
#[case::catalog_model_list(7)]
#[case::auth_missing_401(8)]
#[case::conversation_betas(9)]
#[case::catalog_created_at_default(10)]
#[case::header_merge_precedence(11)]
fn headers_cases(#[case] case: u8) {
    match case {
        0 => default_native_version_case(),
        1 => api_key_and_bearer_case(),
        2 => oauth_beta_merge_case(),
        3 => derived_beta_union_case(),
        4 => vertex_web_search_beta_case(),
        5 => user_beta_preserved_case(),
        6 => projection::response_header_prefixes_case(),
        7 => catalog_model_list_case(),
        8 => auth_missing_401_case(),
        9 => conversation_betas_case(),
        10 => projection::catalog_created_at_default_case(),
        _ => header_merge_precedence_case(),
    }
}

fn validated(
    messages: &[Json],
    params: &HeaderParams,
    settings: &AnthropicSettings,
) -> AnthropicHeaders {
    validated_with(
        messages,
        params,
        &HeaderLiteLlmParams::default(),
        settings,
        false,
    )
}

fn validated_with(
    messages: &[Json],
    params: &HeaderParams,
    litellm_params: &HeaderLiteLlmParams,
    settings: &AnthropicSettings,
    effort_used: bool,
) -> AnthropicHeaders {
    let caller_headers = IndexMap::new();
    validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params,
            messages,
            optional_params: params,
            effort_used,
        },
        settings,
    )
    .unwrap()
}

fn caller_request<'request>(
    caller_headers: &'request IndexMap<String, String>,
    litellm_params: &'request HeaderLiteLlmParams,
    messages: &'request [Json],
    params: &'request HeaderParams,
) -> EnvironmentRequest<'request> {
    EnvironmentRequest {
        model: MODEL,
        caller_headers,
        api_key: None,
        api_base: None,
        litellm_params,
        messages,
        optional_params: params,
        effort_used: false,
    }
}

fn validated_headers(
    caller_headers: &IndexMap<String, String>,
    params: &HeaderParams,
    litellm_params: &HeaderLiteLlmParams,
    settings: &AnthropicSettings,
    messages: &[Json],
) -> AnthropicHeaders {
    validate_environment(
        &caller_request(caller_headers, litellm_params, messages, params),
        settings,
    )
    .unwrap()
}

fn default_native_version_case() {
    let features = HeaderFeatures::default();
    let defaults = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(
        fixture::header(&defaults, ANTHROPIC_VERSION_HEADER),
        Some(ANTHROPIC_VERSION)
    );
    assert_eq!(
        fixture::header(&defaults, ACCEPT_HEADER),
        Some(JSON_CONTENT_TYPE)
    );
    assert_eq!(
        fixture::header(&defaults, CONTENT_TYPE_HEADER),
        Some(JSON_CONTENT_TYPE)
    );
    assert!(!defaults.values.contains_key(OAUTH_BETA_HEADER_NAME));

    let explicit = get_anthropic_headers(&HeaderRequest {
        anthropic_version: Some("2024-01-01"),
        ..fixture::request(None, None, &features)
    });
    assert_eq!(
        fixture::header(&explicit, ANTHROPIC_VERSION_HEADER),
        Some("2024-01-01")
    );

    // prompt caching and PDF use alone add no beta header
    let messages = vec![
        fixture::json(
            r#"{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}"#,
        ),
        fixture::json(
            r#"{"role":"user","content":[{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"AA=="}}]}"#,
        ),
    ];
    let params = fixture::params();
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let guarded = validated(&messages, &params, &settings);
    assert!(!guarded.values.contains_key(OAUTH_BETA_HEADER_NAME));
}

fn api_key_and_bearer_case() {
    let features = HeaderFeatures::default();
    native_key_headers(&features);
    bearer_token_headers(&features);
    resolved_key_beats_token(&features);
    custom_base_bearer_headers(&features);
    official_base_keeps_key(&features);
    custom_base_without_flag(&features);
    environment_token_headers();
    environment_key_headers();
    caller_key_headers();
    empty_caller_key_headers();
}

fn native_key_headers(features: &HeaderFeatures) {
    let key = fixture::secret("sk-ant-api03-fixture");
    let native = get_anthropic_headers(&fixture::request(Some(&key), None, features));
    assert_eq!(
        fixture::header(&native, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!native.values.contains_key(AUTHORIZATION_HEADER));
}

fn bearer_token_headers(features: &HeaderFeatures) {
    let token = fixture::secret("auth-token-value");
    let bearer = get_anthropic_headers(&fixture::request(None, Some(&token), features));
    assert_eq!(
        fixture::header(&bearer, AUTHORIZATION_HEADER),
        Some("Bearer auth-token-value")
    );
    assert!(!bearer.values.contains_key(X_API_KEY));
}

fn resolved_key_beats_token(features: &HeaderFeatures) {
    let key = fixture::secret("sk-ant-api03-fixture");
    let token = fixture::secret("auth-token-value");
    let both = get_anthropic_headers(&fixture::request(Some(&key), Some(&token), features));
    assert_eq!(
        fixture::header(&both, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!both.values.contains_key(AUTHORIZATION_HEADER));
}

fn custom_base_bearer_headers(features: &HeaderFeatures) {
    let custom_key = fixture::secret("sk-custom-key");
    let custom_base = get_anthropic_headers(&HeaderRequest {
        api_base: Some("https://anthropic-proxy.example"),
        use_bearer_for_custom_base: true,
        ..fixture::request(Some(&custom_key), None, features)
    });
    assert_eq!(
        fixture::header(&custom_base, AUTHORIZATION_HEADER),
        Some("Bearer sk-custom-key")
    );
    assert!(!custom_base.values.contains_key(X_API_KEY));
}

fn official_base_keeps_key(features: &HeaderFeatures) {
    let custom_key = fixture::secret("sk-custom-key");
    let official_base = get_anthropic_headers(&HeaderRequest {
        api_base: Some("https://api.anthropic.com"),
        use_bearer_for_custom_base: true,
        ..fixture::request(Some(&custom_key), None, features)
    });
    assert_eq!(
        fixture::header(&official_base, X_API_KEY),
        Some("sk-custom-key")
    );
}

fn custom_base_without_flag(features: &HeaderFeatures) {
    let key = fixture::secret("sk-ant-api03-fixture");
    let bearer_off = get_anthropic_headers(&HeaderRequest {
        api_base: Some("https://anthropic-proxy.example"),
        ..fixture::request(Some(&key), None, features)
    });
    assert_eq!(
        fixture::header(&bearer_off, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!bearer_off.values.contains_key(AUTHORIZATION_HEADER));
}

fn environment_token_headers() {
    let params = fixture::params();
    let messages = Vec::new();
    let token_only = fixture::settings(None, Some("auth-token-value"));
    let resolved_token = validated(&messages, &params, &token_only);
    assert_eq!(
        fixture::header(&resolved_token, AUTHORIZATION_HEADER),
        Some("Bearer auth-token-value")
    );
}

fn environment_key_headers() {
    let params = fixture::params();
    let messages = Vec::new();
    let both_env = fixture::settings(Some("sk-ant-api03-env"), Some("auth-token-value"));
    let resolved_key = validated(&messages, &params, &both_env);
    assert_eq!(
        fixture::header(&resolved_key, X_API_KEY),
        Some("sk-ant-api03-env")
    );
    assert!(!resolved_key.values.contains_key(AUTHORIZATION_HEADER));
}

fn caller_key_headers() {
    let params = fixture::params();
    let messages = Vec::new();
    let settings = fixture::settings(Some("sk-ant-api03-env"), Some("auth-token-value"));
    let caller_key = fixture::secret("sk-ant-api03-caller");
    let caller_headers = IndexMap::new();
    let litellm_params = fixture::litellm_params();
    let mut request = caller_request(&caller_headers, &litellm_params, &messages, &params);
    request.api_key = Some(&caller_key);
    let explicit = validate_environment(&request, &settings).unwrap();
    assert_eq!(
        fixture::header(&explicit, X_API_KEY),
        Some("sk-ant-api03-caller")
    );
}

fn empty_caller_key_headers() {
    let params = fixture::params();
    let messages = Vec::new();
    let empty_key = fixture::secret("");
    let caller_headers = IndexMap::new();
    let litellm_params = fixture::litellm_params();
    let mut request = caller_request(&caller_headers, &litellm_params, &messages, &params);
    request.api_key = Some(&empty_key);
    let token_only = fixture::settings(None, Some("auth-token-value"));
    let empty = validate_environment(&request, &token_only).unwrap();
    assert_eq!(fixture::header(&empty, X_API_KEY), Some(""));
    assert!(!empty.values.contains_key(AUTHORIZATION_HEADER));
}

fn oauth_beta_merge_case() {
    oauth_key_headers();
    plain_key_headers();
    environment_oauth_headers();
    caller_beta_merge_headers();
}

fn oauth_key_headers() {
    let features = HeaderFeatures::default();
    let oauth_key = fixture::secret("sk-ant-oat-fixture-token");
    let user_betas = vec![
        "custom-beta".to_owned(),
        OAUTH_BETA_HEADER.to_owned(),
        "custom-beta".to_owned(),
    ];
    let headers = get_anthropic_headers(&HeaderRequest {
        user_betas: Some(&user_betas),
        ..fixture::request(Some(&oauth_key), None, &features)
    });
    assert_eq!(
        fixture::header(&headers, AUTHORIZATION_HEADER),
        Some("Bearer sk-ant-oat-fixture-token")
    );
    assert_eq!(
        fixture::header(&headers, BROWSER_ACCESS_HEADER),
        Some("true")
    );
    assert!(!headers.values.contains_key(X_API_KEY));
    assert_eq!(
        fixture::beta_tokens(&headers),
        BTreeSet::from(["custom-beta", OAUTH_BETA_HEADER])
    );
}

fn plain_key_headers() {
    let features = HeaderFeatures::default();
    let plain_key = fixture::secret("sk-ant-api03-fixture");
    let plain = get_anthropic_headers(&fixture::request(Some(&plain_key), None, &features));
    assert!(!plain.values.contains_key(BROWSER_ACCESS_HEADER));
    assert!(!plain.values.contains_key(OAUTH_BETA_HEADER_NAME));
    assert!(!plain.values.contains_key(AUTHORIZATION_HEADER));
}

fn environment_oauth_headers() {
    let settings = fixture::settings(Some("sk-ant-oat-fixture-token"), None);
    let params = fixture::params();
    let messages = Vec::new();
    let from_environment = validated(&messages, &params, &settings);
    assert_eq!(
        fixture::header(&from_environment, AUTHORIZATION_HEADER),
        Some("Bearer sk-ant-oat-fixture-token")
    );
    assert_eq!(
        fixture::beta_tokens(&from_environment),
        BTreeSet::from([OAUTH_BETA_HEADER])
    );
}

fn caller_beta_merge_headers() {
    let settings = fixture::settings(Some("sk-ant-oat-fixture-token"), None);
    let params = fixture::params();
    let messages = Vec::new();
    let caller_headers = fixture::headers(&[(OAUTH_BETA_HEADER_NAME, "caller-beta")]);
    let litellm_params = fixture::litellm_params();
    let merged = validated_headers(
        &caller_headers,
        &params,
        &litellm_params,
        &settings,
        &messages,
    );
    assert_eq!(
        fixture::beta_tokens(&merged),
        BTreeSet::from(["caller-beta", OAUTH_BETA_HEADER])
    );
}

fn derived_beta_union_case() {
    assert_feature_betas(
        &[HeaderFeature::FileId],
        &[FILES_API_BETA, FILE_CODE_EXECUTION_BETA],
    );
    assert_feature_betas(&[HeaderFeature::McpServer], &[MCP_CLIENT_BETA]);
    assert_feature_betas(&[HeaderFeature::ToolSearch], &[TOOL_SEARCH_BETA]);
    assert_feature_betas(
        &[HeaderFeature::ProgrammaticToolCalling],
        &[TOOL_SEARCH_BETA],
    );
    assert_feature_betas(&[HeaderFeature::InputExamples], &[TOOL_SEARCH_BETA]);
    assert_feature_betas(&[HeaderFeature::Effort], &[EFFORT_BETA]);
    assert_feature_betas(&[HeaderFeature::CodeExecution], &[CODE_EXECUTION_BETA]);
    assert_feature_betas(&[HeaderFeature::Skills], &[SKILLS_BETA]);
    all_feature_betas();
    detected_feature_betas();
}

fn assert_feature_betas(features: &[HeaderFeature], expected: &[&str]) {
    let detected = fixture::features(features);
    let headers = get_anthropic_headers(&fixture::request(None, None, &detected));
    let actual = fixture::beta_tokens(&headers);
    let expected_set: BTreeSet<&str> = expected.iter().copied().collect();
    assert_eq!(actual, expected_set);
}

fn all_feature_betas() {
    assert_feature_betas(
        &[
            HeaderFeature::FileId,
            HeaderFeature::McpServer,
            HeaderFeature::ToolSearch,
            HeaderFeature::ProgrammaticToolCalling,
            HeaderFeature::InputExamples,
            HeaderFeature::Effort,
            HeaderFeature::CodeExecution,
            HeaderFeature::Skills,
        ],
        &[
            EFFORT_BETA,
            FILE_CODE_EXECUTION_BETA,
            FILES_API_BETA,
            CODE_EXECUTION_BETA,
            MCP_CLIENT_BETA,
            SKILLS_BETA,
            TOOL_SEARCH_BETA,
        ],
    );
}

fn detected_feature_betas() {
    let messages = Vec::new();
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let code_params = fixture::params_from(r#"{"tools":[{"type":"code_execution_20250825"}]}"#);
    let code = validated(&messages, &code_params, &settings);
    assert_eq!(
        fixture::beta_tokens(&code),
        BTreeSet::from([CODE_EXECUTION_BETA])
    );

    let skills_params = fixture::params_from(r#"{"container":{"skills":["xlsx"]}}"#);
    let skills = validated(&messages, &skills_params, &settings);
    assert_eq!(fixture::beta_tokens(&skills), BTreeSet::from([SKILLS_BETA]));

    let effort_used = validated_with(
        &messages,
        &fixture::params(),
        &HeaderLiteLlmParams::default(),
        &settings,
        true,
    );
    assert_eq!(
        fixture::beta_tokens(&effort_used),
        BTreeSet::from([EFFORT_BETA])
    );
}

fn vertex_web_search_beta_case() {
    vertex_headers();
    native_web_search_headers();
    detected_vertex_headers();
}

fn vertex_headers() {
    let features = HeaderFeatures {
        features: BTreeSet::from([HeaderFeature::FileId]),
        computer_tool_used: Some("computer_20250124".to_owned()),
        is_vertex_request: true,
        web_search_tool_used: true,
    };
    let user_betas = vec!["custom-beta".to_owned()];
    let vertex = get_anthropic_headers(&HeaderRequest {
        user_betas: Some(&user_betas),
        ..fixture::request(None, None, &features)
    });
    assert_eq!(
        fixture::header(&vertex, OAUTH_BETA_HEADER_NAME),
        Some(WEB_SEARCH_BETA)
    );

    let hidden_features = HeaderFeatures {
        features: BTreeSet::from([HeaderFeature::FileId]),
        computer_tool_used: Some("computer_20250124".to_owned()),
        is_vertex_request: true,
        web_search_tool_used: false,
    };
    let hidden = get_anthropic_headers(&fixture::request(None, None, &hidden_features));
    assert!(!hidden.values.contains_key(OAUTH_BETA_HEADER_NAME));
}

fn native_web_search_headers() {
    let native_features = HeaderFeatures {
        features: BTreeSet::new(),
        computer_tool_used: None,
        is_vertex_request: false,
        web_search_tool_used: true,
    };
    let native = get_anthropic_headers(&fixture::request(None, None, &native_features));
    assert!(!native.values.contains_key(OAUTH_BETA_HEADER_NAME));

    // an empty computer tool name adds no computer-use beta
    let empty_tool_features = HeaderFeatures {
        features: BTreeSet::new(),
        computer_tool_used: Some(String::new()),
        is_vertex_request: false,
        web_search_tool_used: false,
    };
    let empty_tool = get_anthropic_headers(&fixture::request(None, None, &empty_tool_features));
    assert!(!empty_tool.values.contains_key(OAUTH_BETA_HEADER_NAME));

    let derived_features = HeaderFeatures {
        features: BTreeSet::from([HeaderFeature::FileId]),
        computer_tool_used: None,
        is_vertex_request: false,
        web_search_tool_used: true,
    };
    let native_derived = get_anthropic_headers(&fixture::request(None, None, &derived_features));
    assert_eq!(
        fixture::beta_tokens(&native_derived),
        BTreeSet::from([FILES_API_BETA, FILE_CODE_EXECUTION_BETA])
    );
}

fn detected_vertex_headers() {
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let messages = Vec::new();
    let params = fixture::params_from(
        r#"{"tools":[{"type":"web_search_20250305"}],"is_vertex_request":true}"#,
    );
    let detected = validated(&messages, &params, &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([WEB_SEARCH_BETA])
    );
}

fn user_beta_preserved_case() {
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let params = fixture::params();
    let litellm_params = fixture::litellm_params();
    let messages = Vec::new();
    caller_betas_case(&settings, &params, &litellm_params, &messages);
    spaced_betas_case(&settings, &params, &litellm_params, &messages);
    file_betas_case(&settings, &params, &litellm_params);
    absent_betas_case(&settings, &params, &litellm_params);
}

fn caller_betas_case(
    settings: &AnthropicSettings,
    params: &HeaderParams,
    litellm_params: &HeaderLiteLlmParams,
    messages: &[Json],
) {
    let caller_headers = fixture::headers(&[(OAUTH_BETA_HEADER_NAME, "custom-one,custom-two")]);
    let merged = validated_headers(&caller_headers, params, litellm_params, settings, messages);
    assert_eq!(
        fixture::beta_tokens(&merged),
        BTreeSet::from(["custom-one", "custom-two"])
    );
}

fn spaced_betas_case(
    settings: &AnthropicSettings,
    params: &HeaderParams,
    litellm_params: &HeaderLiteLlmParams,
    messages: &[Json],
) {
    // spelling, padding and empty tokens survive, and a repeated token collapses
    let caller_headers = fixture::headers(&[(OAUTH_BETA_HEADER_NAME, " beta-b ,alpha ,, beta-b ")]);
    let preserved = validated_headers(&caller_headers, params, litellm_params, settings, messages);
    assert_eq!(
        fixture::header(&preserved, OAUTH_BETA_HEADER_NAME),
        Some(", beta-b ,alpha ")
    );
    assert_eq!(
        fixture::beta_tokens(&preserved),
        BTreeSet::from(["", " beta-b ", "alpha "])
    );
}

fn file_betas_case(
    settings: &AnthropicSettings,
    params: &HeaderParams,
    litellm_params: &HeaderLiteLlmParams,
) {
    let messages = vec![fixture::json(
        r#"{"role":"user","content":[{"type":"file","file":{"file_id":"file_1"}}]}"#,
    )];
    let caller_headers = fixture::headers(&[(OAUTH_BETA_HEADER_NAME, "custom-beta")]);
    let combined = validated_headers(&caller_headers, params, litellm_params, settings, &messages);
    assert_eq!(
        fixture::beta_tokens(&combined),
        BTreeSet::from([FILE_CODE_EXECUTION_BETA, FILES_API_BETA, "custom-beta",])
    );
}

fn absent_betas_case(
    settings: &AnthropicSettings,
    params: &HeaderParams,
    litellm_params: &HeaderLiteLlmParams,
) {
    let caller_headers = IndexMap::new();
    let messages = Vec::new();
    let plain = validated_headers(&caller_headers, params, litellm_params, settings, &messages);
    assert!(!plain.values.contains_key(OAUTH_BETA_HEADER_NAME));
}

fn catalog_model_list_case() {
    let models = catalog_models();
    let response = catalog_response(&models);
    assert!(!response.has_more);
    assert_eq!(response.first_id.as_deref(), Some("sonnet-listed"));
    assert_eq!(response.last_id.as_deref(), Some("anthropic/claude-opus-4"));
    assert_eq!(response.data.len(), 2);
    listed_entry_fields(&response);
    empty_replacement_entry(&response);
    display_names_keyed_by_source(&models);
    empty_catalog_response();
}

fn catalog_models() -> Vec<ModelRecord> {
    vec![
        ModelRecord {
            id: "anthropic/claude-sonnet-4".to_owned(),
            max_input_tokens: Some(200_000),
            max_output_tokens: Some(64_000),
        },
        ModelRecord {
            id: "anthropic/claude-opus-4".to_owned(),
            max_input_tokens: None,
            max_output_tokens: None,
        },
    ]
}

fn catalog_response(models: &[ModelRecord]) -> ModelListResponse {
    let display_names = BTreeMap::from([(
        "anthropic/claude-sonnet-4".to_owned(),
        "Claude Sonnet 4".to_owned(),
    )]);
    let listed_ids = BTreeMap::from([
        (
            "anthropic/claude-sonnet-4".to_owned(),
            "sonnet-listed".to_owned(),
        ),
        ("anthropic/claude-opus-4".to_owned(), String::new()),
    ]);
    anthropic_model_list_response(models, &display_names, &listed_ids, "2023-02-28T18:56:42Z")
}

fn listed_entry_fields(response: &ModelListResponse) {
    let listed = response.data.first().unwrap();
    assert_eq!(listed.kind, "model");
    assert_eq!(listed.id, "sonnet-listed");
    assert_eq!(
        listed.source_model.as_deref(),
        Some("anthropic/claude-sonnet-4")
    );
    assert_eq!(listed.display_name, "Claude Sonnet 4");
    assert_eq!(listed.created_at, "2023-02-28T18:56:42Z");
    assert_eq!(listed.max_input_tokens, Some(200_000));
    assert_eq!(listed.max_output_tokens, Some(64_000));
}

fn empty_replacement_entry(response: &ModelListResponse) {
    // an empty replacement still records the source model but falls back to the original id
    let empty_replacement = &response.data[1];
    assert_eq!(empty_replacement.id, "anthropic/claude-opus-4");
    assert_eq!(
        empty_replacement.source_model.as_deref(),
        Some("anthropic/claude-opus-4")
    );
    assert_eq!(empty_replacement.display_name, "anthropic/claude-opus-4");
    assert_eq!(empty_replacement.max_input_tokens, None);
    assert_eq!(empty_replacement.max_output_tokens, None);
}

fn display_names_keyed_by_source(models: &[ModelRecord]) {
    let listed_display = BTreeMap::from([("sonnet-listed".to_owned(), "Wrong name".to_owned())]);
    let keyed = anthropic_model_list_response(
        &models[..1],
        &listed_display,
        &BTreeMap::from([(
            "anthropic/claude-sonnet-4".to_owned(),
            "sonnet-listed".to_owned(),
        )]),
        "2023-02-28T18:56:42Z",
    );
    assert_eq!(
        keyed.data.first().unwrap().display_name,
        "anthropic/claude-sonnet-4"
    );
}

fn empty_catalog_response() {
    let empty = anthropic_model_list_response(&[], &BTreeMap::new(), &BTreeMap::new(), "now");
    assert!(empty.data.is_empty());
    assert!(!empty.has_more);
    assert!(empty.first_id.is_none());
    assert!(empty.last_id.is_none());
}

fn auth_missing_401_case() {
    let error = missing_auth_error();
    assert_eq!(error.status_code(), Some(401));
    let message = error.to_string();
    assert!(message.contains("ANTHROPIC_API_KEY"));
    assert!(message.contains("ANTHROPIC_AUTH_TOKEN"));
    assert!(matches!(
        error,
        HeadersError::Authentication { model } if model == MODEL
    ));

    oauth_header_resolves();
}

fn missing_auth_error() -> HeadersError {
    let settings = fixture::settings(None, None);
    let params = fixture::params();
    let litellm_params = fixture::litellm_params();
    let messages = Vec::new();
    let caller_headers = IndexMap::new();
    validate_environment(
        &caller_request(&caller_headers, &litellm_params, &messages, &params),
        &settings,
    )
    .unwrap_err()
}

fn oauth_header_resolves() {
    let settings = fixture::settings(None, None);
    let params = fixture::params();
    let litellm_params = fixture::litellm_params();
    let messages = Vec::new();
    // an oauth token in the caller headers resolves without any environment key
    let caller_headers = fixture::headers(&[("authorization", "Bearer sk-ant-oat-header-token")]);
    let resolved = validated_headers(
        &caller_headers,
        &params,
        &litellm_params,
        &settings,
        &messages,
    );
    assert_eq!(
        fixture::header(&resolved, AUTHORIZATION_HEADER),
        Some("Bearer sk-ant-oat-header-token")
    );
    assert!(resolved.values.contains_key(BROWSER_ACCESS_HEADER));
}

fn conversation_betas_case() {
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    output_config_betas(&settings);
    thinking_display_betas(&settings);
    tool_change_betas(&settings);
    computer_tool_betas(&settings);
    mcp_server_betas(&settings);
    combined_betas(&settings);
}

fn output_config_betas(settings: &AnthropicSettings) {
    let messages = vec![fixture::json(
        r#"{"role":"user","content":"hi","output_config":{"effort":"high"}}"#,
    )];
    let detected = validated(&messages, &fixture::params(), settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([OUTPUT_CONFIG_BETA])
    );
}

fn thinking_display_betas(settings: &AnthropicSettings) {
    let adaptive = fixture::params_from(r#"{"thinking":{"type":"adaptive","display":"updates"}}"#);
    let adaptive_headers = validated(&[], &adaptive, settings);
    assert_eq!(
        fixture::beta_tokens(&adaptive_headers),
        BTreeSet::from([THINKING_DISPLAY_BETA])
    );

    let enabled = fixture::params_from(r#"{"thinking":{"type":"enabled","display":"updates"}}"#);
    let enabled_headers = validated(&[], &enabled, settings);
    assert_eq!(
        fixture::beta_tokens(&enabled_headers),
        BTreeSet::from([THINKING_DISPLAY_BETA])
    );

    let disabled = fixture::params_from(r#"{"thinking":{"type":"disabled","display":"updates"}}"#);
    let disabled_headers = validated(&[], &disabled, settings);
    assert!(!disabled_headers.values.contains_key(OAUTH_BETA_HEADER_NAME));
}

fn tool_change_betas(settings: &AnthropicSettings) {
    let messages = vec![fixture::json(
        r#"{"role":"system","content":[{"type":"tool_addition","tool":{"type":"tool_reference"}}]}"#,
    )];
    let detected = validated(&messages, &fixture::params(), settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([TOOL_CHANGES_BETA])
    );
}

fn computer_tool_betas(settings: &AnthropicSettings) {
    let new_version = fixture::params_from(r#"{"tools":[{"type":"computer_20250124"}]}"#);
    let new_headers = validated(&[], &new_version, settings);
    assert_eq!(
        fixture::beta_tokens(&new_headers),
        BTreeSet::from(["computer-use-2025-01-24"])
    );

    let old_version = fixture::params_from(r#"{"tools":[{"type":"computer_20241022"}]}"#);
    let old_headers = validated(&[], &old_version, settings);
    assert_eq!(
        fixture::beta_tokens(&old_headers),
        BTreeSet::from(["computer-use-2024-10-22"])
    );

    let custom = fixture::params_from(r#"{"tools":[{"type":"custom_20250101"}]}"#);
    let custom_headers = validated(&[], &custom, settings);
    assert!(!custom_headers.values.contains_key(OAUTH_BETA_HEADER_NAME));
}

fn mcp_server_betas(settings: &AnthropicSettings) {
    let servers = fixture::params_from(r#"{"mcp_servers":[{"type":"url","name":"docs"}]}"#);
    let with_servers = validated(&[], &servers, settings);
    assert_eq!(
        fixture::beta_tokens(&with_servers),
        BTreeSet::from([MCP_CLIENT_BETA])
    );

    let empty = fixture::params_from(r#"{"mcp_servers":[]}"#);
    let empty_servers = validated(&[], &empty, settings);
    assert!(!empty_servers.values.contains_key(OAUTH_BETA_HEADER_NAME));
}

fn combined_betas(settings: &AnthropicSettings) {
    let messages = vec![fixture::json(
        r#"{"role":"user","content":"hi","output_config":{"effort":"high"}}"#,
    )];
    let params = fixture::params_from(r#"{"thinking":{"type":"adaptive","display":"updates"}}"#);
    let combined = validated(&messages, &params, settings);
    assert_eq!(
        fixture::beta_tokens(&combined),
        BTreeSet::from([OUTPUT_CONFIG_BETA, THINKING_DISPLAY_BETA])
    );
}

fn header_merge_precedence_case() {
    merged_headers_apply_caller_and_base();
    explicit_api_base_wins();
    explicit_base_without_bearer();
    bearer_flag_off_keeps_key();
}

fn precedence_caller_headers() -> IndexMap<String, String> {
    fixture::headers(&[
        (ANTHROPIC_VERSION_HEADER, "2020-01-01"),
        (ACCEPT_HEADER, "text/plain"),
        (CONTENT_TYPE_HEADER, "text/plain"),
        ("x-kept", "yes"),
        (OAUTH_BETA_HEADER_NAME, "caller-beta"),
    ])
}

fn custom_base_params() -> HeaderLiteLlmParams {
    HeaderLiteLlmParams {
        api_base: Some("https://anthropic-proxy.example".to_owned()),
        use_bearer_for_custom_base: true,
    }
}

fn merged_headers_apply_caller_and_base() {
    let params = fixture::params();
    let messages = Vec::new();
    let caller_headers = precedence_caller_headers();
    let litellm_params = custom_base_params();
    let custom_settings = fixture::settings(Some("sk-custom-key"), None);
    let merged = validated_headers(
        &caller_headers,
        &params,
        &litellm_params,
        &custom_settings,
        &messages,
    );
    assert_eq!(
        fixture::header(&merged, ANTHROPIC_VERSION_HEADER),
        Some(ANTHROPIC_VERSION)
    );
    assert_eq!(
        fixture::header(&merged, ACCEPT_HEADER),
        Some(JSON_CONTENT_TYPE)
    );
    assert_eq!(
        fixture::header(&merged, CONTENT_TYPE_HEADER),
        Some(JSON_CONTENT_TYPE)
    );
    assert_eq!(fixture::header(&merged, "x-kept"), Some("yes"));
    assert_eq!(
        fixture::beta_tokens(&merged),
        BTreeSet::from(["caller-beta"])
    );
    // the custom base from litellm_params applies the bearer policy to a key without the sk-ant- prefix
    assert_eq!(
        fixture::header(&merged, AUTHORIZATION_HEADER),
        Some("Bearer sk-custom-key")
    );
    assert!(!merged.values.contains_key(X_API_KEY));
}

fn explicit_api_base_wins() {
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let params = fixture::params();
    let messages = Vec::new();
    let caller_headers = precedence_caller_headers();
    let litellm_params = custom_base_params();
    let mut request = caller_request(&caller_headers, &litellm_params, &messages, &params);
    request.api_base = Some("https://api.anthropic.com");
    let explicit = validate_environment(&request, &settings).unwrap();
    assert_eq!(
        fixture::header(&explicit, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!explicit.values.contains_key(AUTHORIZATION_HEADER));
}

fn explicit_base_without_bearer() {
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let params = fixture::params();
    let messages = Vec::new();
    let caller_headers = precedence_caller_headers();
    let default_params = HeaderLiteLlmParams::default();
    let mut request = caller_request(&caller_headers, &default_params, &messages, &params);
    request.api_base = Some("https://anthropic-proxy.example");
    let custom_explicit = validate_environment(&request, &settings).unwrap();
    assert_eq!(
        fixture::header(&custom_explicit, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
}

fn bearer_flag_off_keeps_key() {
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let params = fixture::params();
    let messages = Vec::new();
    let caller_headers = precedence_caller_headers();
    let bearer_off = HeaderLiteLlmParams {
        api_base: Some("https://anthropic-proxy.example".to_owned()),
        use_bearer_for_custom_base: false,
    };
    let mut request = caller_request(&caller_headers, &bearer_off, &messages, &params);
    request.api_base = Some("https://anthropic-proxy.example");
    let plain = validate_environment(&request, &settings).unwrap();
    assert_eq!(
        fixture::header(&plain, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!plain.values.contains_key(AUTHORIZATION_HEADER));
}
