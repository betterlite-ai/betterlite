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
use crate::headers::catalog::{
    ModelRecord, anthropic_model_entry, create_anthropic_model_list_response,
};
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
    let key = fixture::secret("sk-ant-api03-fixture");
    let token = fixture::secret("auth-token-value");
    let native = get_anthropic_headers(&fixture::request(Some(&key), None, &features));
    assert_eq!(
        fixture::header(&native, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!native.values.contains_key(AUTHORIZATION_HEADER));

    let bearer = get_anthropic_headers(&fixture::request(None, Some(&token), &features));
    assert_eq!(
        fixture::header(&bearer, AUTHORIZATION_HEADER),
        Some("Bearer auth-token-value")
    );
    assert!(!bearer.values.contains_key(X_API_KEY));

    // a resolved key wins over the auth token
    let both = get_anthropic_headers(&fixture::request(Some(&key), Some(&token), &features));
    assert_eq!(
        fixture::header(&both, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!both.values.contains_key(AUTHORIZATION_HEADER));

    let custom_base = get_anthropic_headers(&HeaderRequest {
        api_base: Some("https://anthropic-proxy.example"),
        use_bearer_for_custom_base: true,
        ..fixture::request(Some(&key), None, &features)
    });
    assert_eq!(
        fixture::header(&custom_base, AUTHORIZATION_HEADER),
        Some("Bearer sk-ant-api03-fixture")
    );
    assert!(!custom_base.values.contains_key(X_API_KEY));

    let official_base = get_anthropic_headers(&HeaderRequest {
        api_base: Some("https://api.anthropic.com"),
        use_bearer_for_custom_base: true,
        ..fixture::request(Some(&key), None, &features)
    });
    assert_eq!(
        fixture::header(&official_base, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );

    let bearer_off = get_anthropic_headers(&HeaderRequest {
        api_base: Some("https://anthropic-proxy.example"),
        ..fixture::request(Some(&key), None, &features)
    });
    assert_eq!(
        fixture::header(&bearer_off, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!bearer_off.values.contains_key(AUTHORIZATION_HEADER));

    let params = fixture::params();
    let messages = Vec::new();
    let token_only = fixture::settings(None, Some("auth-token-value"));
    let resolved_token = validated(&messages, &params, &token_only);
    assert_eq!(
        fixture::header(&resolved_token, AUTHORIZATION_HEADER),
        Some("Bearer auth-token-value")
    );

    let both_env = fixture::settings(Some("sk-ant-api03-env"), Some("auth-token-value"));
    let resolved_key = validated(&messages, &params, &both_env);
    assert_eq!(
        fixture::header(&resolved_key, X_API_KEY),
        Some("sk-ant-api03-env")
    );
    assert!(!resolved_key.values.contains_key(AUTHORIZATION_HEADER));

    let caller_key = fixture::secret("sk-ant-api03-caller");
    let caller_headers = IndexMap::new();
    let litellm_params = fixture::litellm_params();
    let explicit = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: Some(&caller_key),
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &both_env,
    )
    .unwrap();
    assert_eq!(
        fixture::header(&explicit, X_API_KEY),
        Some("sk-ant-api03-caller")
    );

    // an empty key is resolved, so it blocks the auth-token fallback
    let empty_key = fixture::secret("");
    let empty = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: Some(&empty_key),
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &fixture::settings(None, Some("auth-token-value")),
    )
    .unwrap();
    assert_eq!(fixture::header(&empty, X_API_KEY), Some(""));
    assert!(!empty.values.contains_key(AUTHORIZATION_HEADER));
}

fn oauth_beta_merge_case() {
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

    let plain_key = fixture::secret("sk-ant-api03-fixture");
    let plain = get_anthropic_headers(&fixture::request(Some(&plain_key), None, &features));
    assert!(!plain.values.contains_key(BROWSER_ACCESS_HEADER));
    assert!(!plain.values.contains_key(OAUTH_BETA_HEADER_NAME));
    assert!(!plain.values.contains_key(AUTHORIZATION_HEADER));

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

    // the caller's beta header is merged and deduplicated
    let caller_headers = fixture::headers(&[(OAUTH_BETA_HEADER_NAME, "caller-beta")]);
    let litellm_params = fixture::litellm_params();
    let merged = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
    assert_eq!(
        fixture::beta_tokens(&merged),
        BTreeSet::from(["caller-beta", OAUTH_BETA_HEADER])
    );
}

fn derived_beta_union_case() {
    let features = fixture::features(&[HeaderFeature::FileId]);
    let file_id = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(
        fixture::beta_tokens(&file_id),
        BTreeSet::from([FILES_API_BETA, FILE_CODE_EXECUTION_BETA])
    );

    let features = fixture::features(&[HeaderFeature::McpServer]);
    let mcp = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(
        fixture::beta_tokens(&mcp),
        BTreeSet::from([MCP_CLIENT_BETA])
    );

    let features = fixture::features(&[HeaderFeature::ToolSearch]);
    let tool_search = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(
        fixture::beta_tokens(&tool_search),
        BTreeSet::from([TOOL_SEARCH_BETA])
    );

    let features = fixture::features(&[HeaderFeature::ProgrammaticToolCalling]);
    let programmatic = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(
        fixture::beta_tokens(&programmatic),
        BTreeSet::from([TOOL_SEARCH_BETA])
    );

    let features = fixture::features(&[HeaderFeature::InputExamples]);
    let input_examples = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(
        fixture::beta_tokens(&input_examples),
        BTreeSet::from([TOOL_SEARCH_BETA])
    );

    let features = fixture::features(&[HeaderFeature::Effort]);
    let effort = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(fixture::beta_tokens(&effort), BTreeSet::from([EFFORT_BETA]));

    let features = fixture::features(&[HeaderFeature::CodeExecution]);
    let code_execution = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(
        fixture::beta_tokens(&code_execution),
        BTreeSet::from([CODE_EXECUTION_BETA])
    );

    let features = fixture::features(&[HeaderFeature::Skills]);
    let skills = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(fixture::beta_tokens(&skills), BTreeSet::from([SKILLS_BETA]));

    let features = fixture::features(&[
        HeaderFeature::FileId,
        HeaderFeature::McpServer,
        HeaderFeature::ToolSearch,
        HeaderFeature::ProgrammaticToolCalling,
        HeaderFeature::InputExamples,
        HeaderFeature::Effort,
        HeaderFeature::CodeExecution,
        HeaderFeature::Skills,
    ]);
    let all = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(
        fixture::beta_tokens(&all),
        BTreeSet::from([
            EFFORT_BETA,
            FILE_CODE_EXECUTION_BETA,
            FILES_API_BETA,
            CODE_EXECUTION_BETA,
            MCP_CLIENT_BETA,
            SKILLS_BETA,
            TOOL_SEARCH_BETA,
        ])
    );

    // the message- and tool-derived flags reach get_anthropic_headers through validate_environment
    let params = fixture::params_tools(&["code_execution_20250825"]);
    let messages = Vec::new();
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let detected = validated(&messages, &params, &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([CODE_EXECUTION_BETA])
    );

    let params = fixture::params_container(r#"{"skills":["xlsx"]}"#);
    let detected = validated(&messages, &params, &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([SKILLS_BETA])
    );

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

    let features = HeaderFeatures {
        features: BTreeSet::from([HeaderFeature::FileId]),
        computer_tool_used: Some("computer_20250124".to_owned()),
        is_vertex_request: true,
        web_search_tool_used: false,
    };
    let hidden = get_anthropic_headers(&fixture::request(None, None, &features));
    assert!(!hidden.values.contains_key(OAUTH_BETA_HEADER_NAME));

    let features = HeaderFeatures {
        features: BTreeSet::new(),
        computer_tool_used: None,
        is_vertex_request: false,
        web_search_tool_used: true,
    };
    let native = get_anthropic_headers(&fixture::request(None, None, &features));
    assert!(!native.values.contains_key(OAUTH_BETA_HEADER_NAME));

    // an empty computer tool name adds no computer-use beta
    let features = HeaderFeatures {
        features: BTreeSet::new(),
        computer_tool_used: Some(String::new()),
        is_vertex_request: false,
        web_search_tool_used: false,
    };
    let empty_tool = get_anthropic_headers(&fixture::request(None, None, &features));
    assert!(!empty_tool.values.contains_key(OAUTH_BETA_HEADER_NAME));

    let features = HeaderFeatures {
        features: BTreeSet::from([HeaderFeature::FileId]),
        computer_tool_used: None,
        is_vertex_request: false,
        web_search_tool_used: true,
    };
    let native_derived = get_anthropic_headers(&fixture::request(None, None, &features));
    assert_eq!(
        fixture::beta_tokens(&native_derived),
        BTreeSet::from([FILES_API_BETA, FILE_CODE_EXECUTION_BETA])
    );

    // the vertex flag and the web search flag come from optional_params and tools
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let messages = Vec::new();
    let params = fixture::params_vertex(&["web_search_20250305"]);
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

    let caller_headers = fixture::headers(&[(OAUTH_BETA_HEADER_NAME, "custom-one,custom-two")]);
    let merged = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
    assert_eq!(
        fixture::beta_tokens(&merged),
        BTreeSet::from(["custom-one", "custom-two"])
    );

    // spelling, padding and empty tokens survive, and a repeated token collapses
    let caller_headers = fixture::headers(&[(OAUTH_BETA_HEADER_NAME, " beta-b ,alpha ,, beta-b ")]);
    let preserved = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
    assert_eq!(
        fixture::header(&preserved, OAUTH_BETA_HEADER_NAME),
        Some(", beta-b ,alpha ")
    );
    assert_eq!(
        fixture::beta_tokens(&preserved),
        BTreeSet::from(["", " beta-b ", "alpha "])
    );

    // the user betas join the derived ones
    let messages = vec![fixture::json(
        r#"{"role":"user","content":[{"type":"file","file_id":"file_1"}]}"#,
    )];
    let caller_headers = fixture::headers(&[(OAUTH_BETA_HEADER_NAME, "custom-beta")]);
    let combined = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
    assert_eq!(
        fixture::beta_tokens(&combined),
        BTreeSet::from([FILE_CODE_EXECUTION_BETA, FILES_API_BETA, "custom-beta",])
    );

    // no caller beta header leaves no beta behind
    let caller_headers = IndexMap::new();
    let messages = Vec::new();
    let plain = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
    assert!(!plain.values.contains_key(OAUTH_BETA_HEADER_NAME));
}

fn catalog_model_list_case() {
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
    let response = create_anthropic_model_list_response(
        &models,
        &display_names,
        &listed_ids,
        "2023-02-28T20:16:42Z",
    );
    assert!(!response.has_more);
    assert_eq!(response.first_id.as_deref(), Some("sonnet-listed"));
    assert_eq!(response.last_id.as_deref(), Some("anthropic/claude-opus-4"));
    assert_eq!(response.data.len(), 2);
    let listed = response.data.first().unwrap();
    assert_eq!(listed.kind, "model");
    assert_eq!(listed.id, "sonnet-listed");
    assert_eq!(
        listed.source_model.as_deref(),
        Some("anthropic/claude-sonnet-4")
    );
    assert_eq!(listed.display_name, "Claude Sonnet 4");
    assert_eq!(listed.created_at, "2023-02-28T20:16:42Z");
    assert_eq!(listed.max_input_tokens, Some(200_000));
    assert_eq!(listed.max_output_tokens, Some(64_000));

    // an empty replacement still records the source model but falls back to the original id
    let empty_replacement = response.data.get(1).unwrap();
    assert_eq!(empty_replacement.id, "anthropic/claude-opus-4");
    assert_eq!(
        empty_replacement.source_model.as_deref(),
        Some("anthropic/claude-opus-4")
    );
    assert_eq!(empty_replacement.display_name, "anthropic/claude-opus-4");
    assert_eq!(empty_replacement.max_input_tokens, None);
    assert_eq!(empty_replacement.max_output_tokens, None);

    // display_names are keyed by the source id, not by the listed id
    let listed_display = BTreeMap::from([("sonnet-listed".to_owned(), "Wrong name".to_owned())]);
    let entry = anthropic_model_entry(
        &models.first().unwrap().clone(),
        "2023-02-28T20:16:42Z",
        &listed_display,
        &BTreeMap::from([(
            "anthropic/claude-sonnet-4".to_owned(),
            "sonnet-listed".to_owned(),
        )]),
    );
    assert_eq!(entry.display_name, "anthropic/claude-sonnet-4");

    let empty =
        create_anthropic_model_list_response(&[], &BTreeMap::new(), &BTreeMap::new(), "now");
    assert!(empty.data.is_empty());
    assert!(!empty.has_more);
    assert!(empty.first_id.is_none());
    assert!(empty.last_id.is_none());
}

fn auth_missing_401_case() {
    let settings = fixture::settings(None, None);
    let params = fixture::params();
    let litellm_params = fixture::litellm_params();
    let messages = Vec::new();
    let caller_headers = IndexMap::new();
    let error = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap_err();
    assert_eq!(error.status_code(), Some(401));
    let message = error.to_string();
    assert!(message.contains("ANTHROPIC_API_KEY"));
    assert!(message.contains("ANTHROPIC_AUTH_TOKEN"));
    assert!(matches!(
        error,
        HeadersError::Authentication { ref model } if model == MODEL
    ));

    // an oauth token in the caller headers resolves without any environment key
    let caller_headers = fixture::headers(&[("authorization", "Bearer sk-ant-oat-header-token")]);
    let resolved = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
    assert_eq!(
        fixture::header(&resolved, AUTHORIZATION_HEADER),
        Some("Bearer sk-ant-oat-header-token")
    );
    assert!(resolved.values.contains_key(BROWSER_ACCESS_HEADER));
}

fn conversation_betas_case() {
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);

    let messages = vec![fixture::json(
        r#"{"role":"user","content":"hi","output_config":{"effort":"high"}}"#,
    )];
    let detected = validated(&messages, &fixture::params(), &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([OUTPUT_CONFIG_BETA])
    );

    let params = fixture::params_thinking(r#"{"type":"adaptive","display":"updates"}"#);
    let detected = validated(&[], &params, &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([THINKING_DISPLAY_BETA])
    );

    let params = fixture::params_thinking(r#"{"type":"enabled","display":"updates"}"#);
    let detected = validated(&[], &params, &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([THINKING_DISPLAY_BETA])
    );

    let params = fixture::params_thinking(r#"{"type":"disabled","display":"updates"}"#);
    let detected = validated(&[], &params, &settings);
    assert!(!detected.values.contains_key(OAUTH_BETA_HEADER_NAME));

    let messages = vec![fixture::json(
        r#"{"role":"system","content":[{"type":"tool_addition","tool":{"type":"tool_reference"}}]}"#,
    )];
    let detected = validated(&messages, &fixture::params(), &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([TOOL_CHANGES_BETA])
    );

    let params = fixture::params_tools(&["computer_20250124"]);
    let detected = validated(&[], &params, &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from(["computer-use-2025-01-24"])
    );

    let params = fixture::params_tools(&["computer_20241022"]);
    let detected = validated(&[], &params, &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from(["computer-use-2024-10-22"])
    );

    let params = fixture::params_tools(&["custom_20250101"]);
    let detected = validated(&[], &params, &settings);
    assert!(!detected.values.contains_key(OAUTH_BETA_HEADER_NAME));

    let params = fixture::params_mcp(&[r#"{"type":"url","name":"docs"}"#]);
    let detected = validated(&[], &params, &settings);
    assert_eq!(
        fixture::beta_tokens(&detected),
        BTreeSet::from([MCP_CLIENT_BETA])
    );

    let params = fixture::params_mcp(&[]);
    let detected = validated(&[], &params, &settings);
    assert!(!detected.values.contains_key(OAUTH_BETA_HEADER_NAME));

    let messages = vec![fixture::json(
        r#"{"role":"user","content":"hi","output_config":{"effort":"high"}}"#,
    )];
    let params = fixture::params_thinking(r#"{"type":"adaptive","display":"updates"}"#);
    let combined = validated(&messages, &params, &settings);
    assert_eq!(
        fixture::beta_tokens(&combined),
        BTreeSet::from([OUTPUT_CONFIG_BETA, THINKING_DISPLAY_BETA])
    );
}

fn header_merge_precedence_case() {
    let settings = fixture::settings(Some("sk-ant-api03-fixture"), None);
    let params = fixture::params();
    let messages = Vec::new();
    let caller_headers = fixture::headers(&[
        (ANTHROPIC_VERSION_HEADER, "2020-01-01"),
        (ACCEPT_HEADER, "text/plain"),
        (CONTENT_TYPE_HEADER, "text/plain"),
        ("x-kept", "yes"),
        (OAUTH_BETA_HEADER_NAME, "caller-beta"),
    ]);
    let litellm_params = HeaderLiteLlmParams {
        api_base: Some("https://anthropic-proxy.example".to_owned()),
        use_bearer_for_custom_base: true,
    };
    let merged = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
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
    // the custom base from litellm_params applies the bearer policy
    assert_eq!(
        fixture::header(&merged, AUTHORIZATION_HEADER),
        Some("Bearer sk-ant-api03-fixture")
    );
    assert!(!merged.values.contains_key(X_API_KEY));

    // an explicit api_base is used instead of litellm_params.api_base
    let explicit = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: Some("https://api.anthropic.com"),
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
    assert_eq!(
        fixture::header(&explicit, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!explicit.values.contains_key(AUTHORIZATION_HEADER));

    // an explicit api_base reaches the bearer policy on its own
    let custom_explicit = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: Some("https://anthropic-proxy.example"),
            litellm_params: &HeaderLiteLlmParams::default(),
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
    assert_eq!(
        fixture::header(&custom_explicit, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );

    // use_bearer_for_custom_base defaults to false
    let bearer_off = HeaderLiteLlmParams {
        api_base: Some("https://anthropic-proxy.example".to_owned()),
        use_bearer_for_custom_base: false,
    };
    let plain = validate_environment(
        &EnvironmentRequest {
            model: MODEL,
            caller_headers: &caller_headers,
            api_key: None,
            api_base: Some("https://anthropic-proxy.example"),
            litellm_params: &bearer_off,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap();
    assert_eq!(
        fixture::header(&plain, X_API_KEY),
        Some("sk-ant-api03-fixture")
    );
    assert!(!plain.values.contains_key(AUTHORIZATION_HEADER));
}
