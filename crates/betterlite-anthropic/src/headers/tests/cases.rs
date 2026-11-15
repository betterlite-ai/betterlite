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
    for scenario in &NATIVE_AUTH_SCENARIOS {
        assert_auth_headers(scenario, &features);
    }
    for scenario in &ENVIRONMENT_AUTH_SCENARIOS {
        assert_environment_auth(scenario);
    }
}

struct AuthHeadersScenario {
    api_key: Option<&'static str>,
    auth_token: Option<&'static str>,
    api_base: Option<&'static str>,
    use_bearer_for_custom_base: bool,
    expected: &'static [(&'static str, &'static str)],
    absent: &'static [&'static str],
}

const NATIVE_AUTH_SCENARIOS: [AuthHeadersScenario; 6] = [
    AuthHeadersScenario {
        api_key: Some("sk-ant-api03-fixture"),
        auth_token: None,
        api_base: None,
        use_bearer_for_custom_base: false,
        expected: &[(X_API_KEY, "sk-ant-api03-fixture")],
        absent: &[AUTHORIZATION_HEADER],
    },
    AuthHeadersScenario {
        api_key: None,
        auth_token: Some("auth-token-value"),
        api_base: None,
        use_bearer_for_custom_base: false,
        expected: &[(AUTHORIZATION_HEADER, "Bearer auth-token-value")],
        absent: &[X_API_KEY],
    },
    AuthHeadersScenario {
        api_key: Some("sk-ant-api03-fixture"),
        auth_token: Some("auth-token-value"),
        api_base: None,
        use_bearer_for_custom_base: false,
        expected: &[(X_API_KEY, "sk-ant-api03-fixture")],
        absent: &[AUTHORIZATION_HEADER],
    },
    AuthHeadersScenario {
        api_key: Some("sk-custom-key"),
        auth_token: None,
        api_base: Some("https://anthropic-proxy.example"),
        use_bearer_for_custom_base: true,
        expected: &[(AUTHORIZATION_HEADER, "Bearer sk-custom-key")],
        absent: &[X_API_KEY],
    },
    AuthHeadersScenario {
        api_key: Some("sk-custom-key"),
        auth_token: None,
        api_base: Some("https://api.anthropic.com"),
        use_bearer_for_custom_base: true,
        expected: &[(X_API_KEY, "sk-custom-key")],
        absent: &[],
    },
    AuthHeadersScenario {
        api_key: Some("sk-ant-api03-fixture"),
        auth_token: None,
        api_base: Some("https://anthropic-proxy.example"),
        use_bearer_for_custom_base: false,
        expected: &[(X_API_KEY, "sk-ant-api03-fixture")],
        absent: &[AUTHORIZATION_HEADER],
    },
];

struct EnvironmentAuthScenario {
    settings_api_key: Option<&'static str>,
    settings_auth_token: Option<&'static str>,
    caller_api_key: Option<&'static str>,
    expected: (&'static str, &'static str),
    absent: &'static [&'static str],
}

const ENVIRONMENT_AUTH_SCENARIOS: [EnvironmentAuthScenario; 4] = [
    EnvironmentAuthScenario {
        settings_api_key: None,
        settings_auth_token: Some("auth-token-value"),
        caller_api_key: None,
        expected: (AUTHORIZATION_HEADER, "Bearer auth-token-value"),
        absent: &[],
    },
    EnvironmentAuthScenario {
        settings_api_key: Some("sk-ant-api03-env"),
        settings_auth_token: Some("auth-token-value"),
        caller_api_key: None,
        expected: (X_API_KEY, "sk-ant-api03-env"),
        absent: &[AUTHORIZATION_HEADER],
    },
    EnvironmentAuthScenario {
        settings_api_key: Some("sk-ant-api03-env"),
        settings_auth_token: Some("auth-token-value"),
        caller_api_key: Some("sk-ant-api03-caller"),
        expected: (X_API_KEY, "sk-ant-api03-caller"),
        absent: &[],
    },
    EnvironmentAuthScenario {
        settings_api_key: None,
        settings_auth_token: Some("auth-token-value"),
        caller_api_key: Some(""),
        expected: (X_API_KEY, ""),
        absent: &[AUTHORIZATION_HEADER],
    },
];

fn assert_auth_headers(scenario: &AuthHeadersScenario, features: &HeaderFeatures) {
    let api_key = scenario.api_key.map(fixture::secret);
    let auth_token = scenario.auth_token.map(fixture::secret);
    let headers = get_anthropic_headers(&HeaderRequest {
        api_key: api_key.as_ref(),
        auth_token: auth_token.as_ref(),
        api_base: scenario.api_base,
        use_bearer_for_custom_base: scenario.use_bearer_for_custom_base,
        ..fixture::request(None, None, features)
    });
    assert_header_expectations(&headers, scenario.expected, scenario.absent);
}

fn assert_environment_auth(scenario: &EnvironmentAuthScenario) {
    let settings = fixture::settings(scenario.settings_api_key, scenario.settings_auth_token);
    let params = fixture::params();
    let messages = Vec::new();
    let caller_headers = IndexMap::new();
    let litellm_params = fixture::litellm_params();
    let caller_key = scenario.caller_api_key.map(fixture::secret);
    let mut request = caller_request(&caller_headers, &litellm_params, &messages, &params);
    request.api_key = caller_key.as_ref();
    let headers = validate_environment(&request, &settings).unwrap();
    assert_header_expectations(&headers, &[scenario.expected], scenario.absent);
}

fn assert_header_expectations(
    headers: &AnthropicHeaders,
    expected: &[(&str, &str)],
    absent: &[&str],
) {
    for &(name, value) in expected {
        assert_eq!(fixture::header(headers, name), Some(value));
    }
    for &name in absent {
        assert!(!headers.values.contains_key(name));
    }
}

fn assert_beta_tokens(headers: &AnthropicHeaders, expected: &[&str]) {
    let expected_set: BTreeSet<&str> = expected.iter().copied().collect();
    assert_eq!(fixture::beta_tokens(headers), expected_set);
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
    for scenario in &CALLER_BETA_SCENARIOS {
        assert_caller_beta_scenario(scenario, &settings, &params, &litellm_params);
    }
}

struct CallerBetaScenario {
    caller_betas: Option<&'static str>,
    message: Option<&'static str>,
    expected_header: Option<&'static str>,
    header_absent: bool,
    expected_tokens: &'static [&'static str],
}

const CALLER_BETA_SCENARIOS: [CallerBetaScenario; 4] = [
    CallerBetaScenario {
        caller_betas: Some("custom-one,custom-two"),
        message: None,
        expected_header: None,
        header_absent: false,
        expected_tokens: &["custom-one", "custom-two"],
    },
    CallerBetaScenario {
        caller_betas: Some(" beta-b ,alpha ,, beta-b "),
        message: None,
        expected_header: Some(", beta-b ,alpha "),
        header_absent: false,
        expected_tokens: &["", " beta-b ", "alpha "],
    },
    CallerBetaScenario {
        caller_betas: Some("custom-beta"),
        message: Some(r#"{"role":"user","content":[{"type":"file","file":{"file_id":"file_1"}}]}"#),
        expected_header: None,
        header_absent: false,
        expected_tokens: &[FILE_CODE_EXECUTION_BETA, FILES_API_BETA, "custom-beta"],
    },
    CallerBetaScenario {
        caller_betas: None,
        message: None,
        expected_header: None,
        header_absent: true,
        expected_tokens: &[],
    },
];

fn assert_caller_beta_scenario(
    scenario: &CallerBetaScenario,
    settings: &AnthropicSettings,
    params: &HeaderParams,
    litellm_params: &HeaderLiteLlmParams,
) {
    let messages: Vec<Json> = scenario.message.map(fixture::json).into_iter().collect();
    let caller_headers = scenario.caller_betas.map_or_else(IndexMap::new, |value| {
        fixture::headers(&[(OAUTH_BETA_HEADER_NAME, value)])
    });
    let headers = validated_headers(&caller_headers, params, litellm_params, settings, &messages);
    if scenario.header_absent {
        assert!(!headers.values.contains_key(OAUTH_BETA_HEADER_NAME));
    } else if let Some(value) = scenario.expected_header {
        assert_eq!(
            fixture::header(&headers, OAUTH_BETA_HEADER_NAME),
            Some(value)
        );
    }
    assert_beta_tokens(&headers, scenario.expected_tokens);
}

fn catalog_model_list_case() {
    let (models, display_names, listed_ids) = catalog_fixture();
    let response =
        anthropic_model_list_response(&models, &display_names, &listed_ids, "2023-02-28T18:56:42Z");
    assert!(!response.has_more);
    assert_eq!(response.first_id.as_deref(), Some("sonnet-listed"));
    assert_eq!(response.last_id.as_deref(), Some("anthropic/claude-opus-4"));
    assert_eq!(response.data.len(), 2);
    listed_entry_fields(&response);
    empty_replacement_entry(&response);
    display_names_keyed_by_source(&models);
    empty_catalog_response();
}

fn catalog_fixture() -> (
    Vec<ModelRecord>,
    BTreeMap<String, String>,
    BTreeMap<String, String>,
) {
    let models = vec![
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
    ];
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
    (models, display_names, listed_ids)
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
    for scenario in &CONVERSATION_BETA_SCENARIOS {
        assert_conversation_beta_scenario(scenario, &settings);
    }
}

struct ConversationBetaScenario {
    message: Option<&'static str>,
    params: Option<&'static str>,
    expected_tokens: &'static [&'static str],
    absent: bool,
}

const CONVERSATION_BETA_SCENARIOS: [ConversationBetaScenario; 11] = [
    ConversationBetaScenario {
        message: Some(r#"{"role":"user","content":"hi","output_config":{"effort":"high"}}"#),
        params: None,
        expected_tokens: &[OUTPUT_CONFIG_BETA],
        absent: false,
    },
    ConversationBetaScenario {
        message: None,
        params: Some(r#"{"thinking":{"type":"adaptive","display":"updates"}}"#),
        expected_tokens: &[THINKING_DISPLAY_BETA],
        absent: false,
    },
    ConversationBetaScenario {
        message: None,
        params: Some(r#"{"thinking":{"type":"enabled","display":"updates"}}"#),
        expected_tokens: &[THINKING_DISPLAY_BETA],
        absent: false,
    },
    ConversationBetaScenario {
        message: None,
        params: Some(r#"{"thinking":{"type":"disabled","display":"updates"}}"#),
        expected_tokens: &[],
        absent: true,
    },
    ConversationBetaScenario {
        message: Some(
            r#"{"role":"system","content":[{"type":"tool_addition","tool":{"type":"tool_reference"}}]}"#,
        ),
        params: None,
        expected_tokens: &[TOOL_CHANGES_BETA],
        absent: false,
    },
    ConversationBetaScenario {
        message: None,
        params: Some(r#"{"tools":[{"type":"computer_20250124"}]}"#),
        expected_tokens: &["computer-use-2025-01-24"],
        absent: false,
    },
    ConversationBetaScenario {
        message: None,
        params: Some(r#"{"tools":[{"type":"computer_20241022"}]}"#),
        expected_tokens: &["computer-use-2024-10-22"],
        absent: false,
    },
    ConversationBetaScenario {
        message: None,
        params: Some(r#"{"tools":[{"type":"custom_20250101"}]}"#),
        expected_tokens: &[],
        absent: true,
    },
    ConversationBetaScenario {
        message: None,
        params: Some(r#"{"mcp_servers":[{"type":"url","name":"docs"}]}"#),
        expected_tokens: &[MCP_CLIENT_BETA],
        absent: false,
    },
    ConversationBetaScenario {
        message: None,
        params: Some(r#"{"mcp_servers":[]}"#),
        expected_tokens: &[],
        absent: true,
    },
    ConversationBetaScenario {
        message: Some(r#"{"role":"user","content":"hi","output_config":{"effort":"high"}}"#),
        params: Some(r#"{"thinking":{"type":"adaptive","display":"updates"}}"#),
        expected_tokens: &[OUTPUT_CONFIG_BETA, THINKING_DISPLAY_BETA],
        absent: false,
    },
];

fn assert_conversation_beta_scenario(
    scenario: &ConversationBetaScenario,
    settings: &AnthropicSettings,
) {
    let messages: Vec<Json> = scenario.message.map(fixture::json).into_iter().collect();
    let params = scenario
        .params
        .map_or_else(fixture::params, fixture::params_from);
    let headers = validated(&messages, &params, settings);
    if scenario.absent {
        assert!(!headers.values.contains_key(OAUTH_BETA_HEADER_NAME));
    } else {
        assert_beta_tokens(&headers, scenario.expected_tokens);
    }
}

fn header_merge_precedence_case() {
    merged_headers_apply_caller_and_base();
    for scenario in &EXPLICIT_BASE_SCENARIOS {
        assert_explicit_base_scenario(scenario);
    }
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

struct ExplicitBaseScenario {
    litellm_api_base: Option<&'static str>,
    litellm_bearer: bool,
    request_api_base: &'static str,
}

const EXPLICIT_BASE_SCENARIOS: [ExplicitBaseScenario; 3] = [
    ExplicitBaseScenario {
        litellm_api_base: Some("https://anthropic-proxy.example"),
        litellm_bearer: true,
        request_api_base: "https://api.anthropic.com",
    },
    ExplicitBaseScenario {
        litellm_api_base: None,
        litellm_bearer: false,
        request_api_base: "https://anthropic-proxy.example",
    },
    ExplicitBaseScenario {
        litellm_api_base: Some("https://anthropic-proxy.example"),
        litellm_bearer: false,
        request_api_base: "https://anthropic-proxy.example",
    },
];

fn assert_explicit_base_scenario(scenario: &ExplicitBaseScenario) {
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let params = fixture::params();
    let messages = Vec::new();
    let caller_headers = precedence_caller_headers();
    let litellm_params = HeaderLiteLlmParams {
        api_base: scenario.litellm_api_base.map(str::to_owned),
        use_bearer_for_custom_base: scenario.litellm_bearer,
    };
    let mut request = caller_request(&caller_headers, &litellm_params, &messages, &params);
    request.api_base = Some(scenario.request_api_base);
    let headers = validate_environment(&request, &settings).unwrap();
    assert_header_expectations(
        &headers,
        &[(X_API_KEY, "sk-ant-api03-fixture")],
        &[AUTHORIZATION_HEADER],
    );
}
