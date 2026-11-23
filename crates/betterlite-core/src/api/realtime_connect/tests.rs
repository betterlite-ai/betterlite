#![cfg(test)]
mod credentials;
mod forwarding;
mod guardrails;
mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use betterlite_config::api::realtime_connect::RealtimeConnectionSettings;
use betterlite_context::resource_config_selection::realtime::RealtimeConfigTag;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::vertex_chirp::target::AccessTokenResolver;

use super::token::{VertexTokenError, resolve_vertex_token};
use super::{
    AzureAdTokenRequest, Branch, ConnectionQuery, RealtimeConnectDispatch as _,
    RealtimeConnectError, RealtimeConnectParams, RealtimeConnection,
};
use support::{
    BareDispatch, Recorder, ScriptedTokens, TestConfig, call, params, run, run_with_environment,
    secret, settings, text,
};

#[tokio::test]
async fn realtime_generic_dispatch() {
    let dispatch = Recorder::default();
    let (logging, connection, caller_query) = generic_connection(&dispatch).await;
    assert_eq!(dispatch.tags(), vec![RealtimeConfigTag::Gemini]);
    assert_generic_arguments(&connection, &caller_query);
    assert_generic_metadata_and_logging(&connection, &logging);
    guardrails::kwargs_guardrails_fallback().await;
}

async fn generic_connection(
    dispatch: &Recorder,
) -> (
    LoggingState,
    RealtimeConnection<TestConfig>,
    IndexMap<String, String>,
) {
    let mut connect = params();
    connect.api_base = Some(String::from("https://named.example/v1"));
    connect.api_key = Some(secret("sk-named"));
    connect.headers = Some(HeaderMap::from([(String::from("x"), text("old"))]));
    connect.extra_headers = Some(HeaderMap::from([
        (String::from("x"), text("new")),
        (String::from("y"), text("test")),
    ]));
    connect.metadata = Some(IndexMap::from([(
        String::from("guardrails"),
        Json::List(vec![text("first")]),
    )]));
    connect.guardrails = Some(Json::List(vec![text("second")]));
    connect.litellm_metadata = Some(IndexMap::from([
        (String::from("guardrails"), Json::List(vec![text("third")])),
        (String::from("trace"), text("t1")),
    ]));
    connect.user = Some(String::from("u-1"));
    let litellm_params = IndexMap::new();
    let caller_query = IndexMap::from([
        (String::from("model"), String::from("alias")),
        (String::from("intent"), String::from("transcription")),
    ]);
    let call = call(
        "gemini/gemini-2.0-flash-live-001",
        &connect,
        Some(&caller_query),
        &litellm_params,
        false,
    );
    let (logging, result) = run(dispatch, &settings(), &call).await;
    (logging, result.unwrap(), caller_query)
}

fn assert_generic_arguments(
    connection: &RealtimeConnection<TestConfig>,
    caller_query: &IndexMap<String, String>,
) {
    let Branch::ProviderConfig { tag, .. } = &connection.branch else {
        panic!("a gemini model must take the generic provider-config branch");
    };
    assert_eq!(*tag, RealtimeConfigTag::Gemini);
    let args = &connection.args;
    assert_eq!(args.api_base.as_deref(), Some("https://named.example/v1"));
    assert_eq!(
        args.api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("sk-named")
    );
    assert_eq!(
        args.headers,
        Some(HeaderMap::from([
            (String::from("x"), text("new")),
            (String::from("y"), text("test")),
        ]))
    );
    let ConnectionQuery::Transport(Some(query)) = &args.query_params else {
        panic!("the generic branch carries the transport query");
    };
    assert_eq!(
        query.get("model"),
        Some(&String::from("gemini-2.0-flash-live-001"))
    );
    assert_eq!(query.get("intent"), Some(&String::from("transcription")));
    assert_eq!(caller_query.get("model").map(String::as_str), Some("alias"));
}

fn assert_generic_metadata_and_logging(
    connection: &RealtimeConnection<TestConfig>,
    logging: &LoggingState,
) {
    let metadata = connection.args.litellm_metadata.as_ref().unwrap();
    assert_eq!(
        metadata.get("guardrails"),
        Some(&Json::List(vec![text("first")]))
    );
    assert_eq!(metadata.get("trace"), Some(&text("t1")));
    assert_eq!(logging.model, "gemini-2.0-flash-live-001");
    assert_eq!(logging.user.as_deref(), Some("u-1"));
    assert_eq!(
        logging.litellm_params.get("_arealtime"),
        Some(&Json::Bool(true))
    );
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("gemini"));
    assert!(logging.optional_params.is_empty());
}

#[tokio::test]
async fn realtime_generic_dispatch_keeps_guardrails_and_reaches_meta() {
    let dispatch = Recorder::default();
    let mut connect = params();
    connect.metadata = Some(IndexMap::from([(
        String::from("guardrails"),
        Json::List(Vec::new()),
    )]));
    connect.guardrails = Some(Json::List(Vec::new()));
    connect.litellm_metadata = Some(IndexMap::from([(
        String::from("guardrails"),
        Json::List(vec![text("third")]),
    )]));
    let litellm_params = IndexMap::new();
    let call = call("meta/muse-1", &connect, None, &litellm_params, false);
    let (_, result) = run(&dispatch, &settings(), &call).await;
    let connection = result.unwrap();
    let Branch::ProviderConfig { tag, .. } = &connection.branch else {
        panic!("a meta model must take the generic provider-config branch");
    };
    assert_eq!(*tag, RealtimeConfigTag::Meta);
    assert_eq!(dispatch.tags(), vec![RealtimeConfigTag::Meta]);
    let metadata = connection.args.litellm_metadata.as_ref().unwrap();
    assert_eq!(
        metadata.get("guardrails"),
        Some(&Json::List(vec![text("third")]))
    );
    assert_eq!(connection.args.api_base, None);
}

#[tokio::test]
async fn realtime_generic_dispatch_rejects_an_unsupported_model() {
    let dispatch = Recorder::default();
    let connect = params();
    let litellm_params = IndexMap::new();
    let call = call("anthropic/claude-x", &connect, None, &litellm_params, false);
    let (_, result) = run(&dispatch, &settings(), &call).await;
    let Err(error) = result else {
        panic!("an unsupported model must fail");
    };
    assert_eq!(error.to_string(), "Unsupported model: claude-x");
    assert!(dispatch.tags().is_empty());
}

#[test]
fn realtime_connect_dispatch_defaults_report_not_built() {
    let dispatch = BareDispatch;
    assert!(matches!(
        dispatch.provider_config(RealtimeConfigTag::Gemini),
        Err(RealtimeConnectError::NotBuilt(_))
    ));
    assert!(matches!(
        dispatch.vertex_access_tokens(),
        Err(RealtimeConnectError::NotBuilt(_))
    ));
    let connect = params();
    assert!(matches!(
        dispatch.azure_ad_token(AzureAdTokenRequest {
            azure_ad_token: None,
            params: &connect,
        }),
        Err(RealtimeConnectError::NotBuilt(_))
    ));
}

#[tokio::test]
async fn realtime_openai_and_azure_credentials() {
    let mut settings = settings();
    settings.api_base = Some(String::from("https://global.example"));
    settings.api_key = Some(secret("sk-global"));
    settings.openai_key = Some(secret("sk-openai-global"));
    settings.azure_api_base = Some(String::from("https://azure.example"));
    settings.azure_api_key = Some(secret("azure-key"));
    settings.openai_api_key = Some(secret("openai-key"));
    let dispatch = Recorder::default();
    let litellm_params = IndexMap::new();
    azure_global_credentials(&dispatch, &settings, &litellm_params).await;
    azure_named_version_and_protocol(&dispatch, &settings, &litellm_params).await;
    azure_ad_token_fallback(&litellm_params).await;
    credentials::openai_credentials(&dispatch, &litellm_params).await;
    named_api_key_wins().await;
    azure_settings_key_fallbacks().await;
    credentials::openai_settings_key_fallbacks().await;
    azure_generic_precedence(&dispatch, &settings, &litellm_params).await;
    azure_protocol_negotiation(&litellm_params).await;
    credentials::openai_generic_base_and_global_key_win().await;
    forwarding::connector_branches_drop_the_client().await;
}

async fn azure_generic_precedence(
    dispatch: &Recorder,
    settings: &RealtimeConnectionSettings,
    litellm_params: &IndexMap<String, Json>,
) {
    let mut connect = params();
    connect.generic_api_base = Some(String::from("https://generic.example"));
    connect.generic_api_version = Some(String::from("2025-06-01"));
    connect.generic_realtime_protocol = Some(String::from("beta"));
    assert_eq!(
        azure_observed(dispatch, settings, &connect, litellm_params).await,
        (
            Some(String::from("https://generic.example")),
            String::from("2025-06-01"),
            String::from("beta")
        )
    );
    azure_environment_protocol(dispatch, settings, litellm_params).await;
}

async fn azure_observed(
    dispatch: &Recorder,
    settings: &RealtimeConnectionSettings,
    connect: &RealtimeConnectParams,
    litellm_params: &IndexMap<String, Json>,
) -> (Option<String>, String, String) {
    let call = call("azure/my-deployment", connect, None, litellm_params, false);
    let (_, result) = run(dispatch, settings, &call).await;
    let connection = result.unwrap();
    let Branch::Azure {
        api_version,
        protocol,
        ..
    } = connection.branch
    else {
        panic!("an azure model must take the azure branch");
    };
    (connection.args.api_base, api_version, protocol)
}

async fn azure_environment_protocol(
    dispatch: &Recorder,
    settings: &RealtimeConnectionSettings,
    litellm_params: &IndexMap<String, Json>,
) {
    let mut configured = settings.clone();
    configured.azure_realtime_protocol = Some(String::from("beta"));
    let connect = params();
    let call = call("azure/my-deployment", &connect, None, litellm_params, false);
    let (_, result) = run(dispatch, &configured, &call).await;
    let Branch::Azure { protocol, .. } = result.unwrap().branch else {
        panic!("an azure model must take the azure branch");
    };
    assert_eq!(protocol, "beta");
}

async fn named_api_key_wins() {
    let dispatch = Recorder::default();
    let mut globals = settings();
    globals.api_key = Some(secret("sk-global"));
    globals.azure_api_key = Some(secret("azure-key"));
    globals.openai_api_key = Some(secret("openai-key"));
    let mut named = params();
    named.api_key = Some(secret("sk-named"));
    assert_api_key(
        &dispatch,
        &globals,
        "azure/my-deployment",
        &named,
        "sk-named",
    )
    .await;
    assert_api_key(
        &dispatch,
        &globals,
        "openai/gpt-realtime",
        &named,
        "sk-named",
    )
    .await;
}

async fn assert_api_key(
    dispatch: &Recorder,
    settings: &RealtimeConnectionSettings,
    model: &str,
    connect: &RealtimeConnectParams,
    expected: &str,
) {
    let litellm_params = IndexMap::new();
    let request = call(model, connect, None, &litellm_params, false);
    let (_, result) = run(dispatch, settings, &request).await;
    let connection = result.unwrap();
    assert_eq!(
        connection
            .args
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some(expected)
    );
}

async fn azure_settings_key_fallbacks() {
    let dispatch = Recorder::default();
    let mut environment_only = settings();
    environment_only.azure_api_key = Some(secret("azure-only-key"));
    assert_api_key(
        &dispatch,
        &environment_only,
        "azure/my-deployment",
        &params(),
        "azure-only-key",
    )
    .await;
    let mut legacy_only = settings();
    legacy_only.openai_key = Some(secret("sk-openai-global"));
    assert_api_key(
        &dispatch,
        &legacy_only,
        "azure/my-deployment",
        &params(),
        "sk-openai-global",
    )
    .await;
}

async fn azure_protocol_negotiation(litellm_params: &IndexMap<String, Json>) {
    assert_eq!(azure_client_protocol(litellm_params, true).await, "beta");
    assert_eq!(azure_client_protocol(litellm_params, false).await, "GA");
}

async fn azure_client_protocol(
    litellm_params: &IndexMap<String, Json>,
    client_sent_beta: bool,
) -> String {
    let dispatch = Recorder::with_ad_token(Some(secret("resolved-ad")));
    let mut settings = settings();
    settings.azure_api_base = Some(String::from("https://azure.example"));
    let connect = params();
    let call = call(
        "azure/my-deployment",
        &connect,
        None,
        litellm_params,
        client_sent_beta,
    );
    let (_, result) = run(&dispatch, &settings, &call).await;
    let Branch::Azure { protocol, .. } = result.unwrap().branch else {
        panic!("an azure model must take the azure branch");
    };
    protocol
}

async fn azure_global_credentials(
    dispatch: &Recorder,
    settings: &RealtimeConnectionSettings,
    litellm_params: &IndexMap<String, Json>,
) {
    let connect = params();
    let query = IndexMap::from([(String::from("intent"), String::from("transcription"))]);
    let call = call(
        "azure/my-deployment",
        &connect,
        Some(&query),
        litellm_params,
        true,
    );
    let (_, result) = run(dispatch, settings, &call).await;
    let connection = result.unwrap();
    assert_azure_global_arguments(&connection);
    assert!(
        dispatch
            .ad_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}

fn assert_azure_global_arguments(connection: &RealtimeConnection<TestConfig>) {
    let Branch::Azure {
        api_version,
        protocol,
        azure_ad_token,
    } = &connection.branch
    else {
        panic!("an azure model must take the azure branch");
    };
    assert_eq!(
        connection.args.api_base.as_deref(),
        Some("https://global.example")
    );
    assert_eq!(
        connection
            .args
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("sk-global")
    );
    assert_eq!(api_version, "2024-10-01-preview");
    assert_eq!(protocol, "GA");
    assert!(azure_ad_token.is_none());
    let ConnectionQuery::Connector(Some(query)) = &connection.args.query_params else {
        panic!("the azure branch carries the connector query");
    };
    assert_eq!(
        query.get("intent"),
        Some(&Some(String::from("transcription")))
    );
    // a transcription-only query stays model-free
    assert!(!query.contains_key("model"));
}

async fn azure_ad_token_fallback(litellm_params: &IndexMap<String, Json>) {
    let mut no_keys = settings();
    no_keys.azure_api_base = Some(String::from("https://azure.example"));
    let mut ad_token = params();
    ad_token.azure_ad_token = Some(secret("named-ad"));
    ad_token.tenant_id = Some(String::from("tenant-1"));
    let dispatch = Recorder::with_ad_token(Some(secret("resolved-ad")));
    let call = call(
        "azure/my-deployment",
        &ad_token,
        None,
        litellm_params,
        false,
    );
    let (_, result) = run(&dispatch, &no_keys, &call).await;
    let connection = result.unwrap();
    let Branch::Azure { azure_ad_token, .. } = &connection.branch else {
        panic!("an azure model must take the azure branch");
    };
    assert_eq!(
        connection.args.api_base.as_deref(),
        Some("https://azure.example")
    );
    assert!(connection.args.api_key.is_none());
    assert_eq!(
        azure_ad_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("resolved-ad")
    );
    assert_azure_ad_request(&dispatch);
}

fn assert_azure_ad_request(dispatch: &Recorder) {
    assert_eq!(
        *dispatch
            .ad_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        vec![(
            Some(String::from("named-ad")),
            Some(String::from("tenant-1"))
        )]
    );
}

async fn azure_named_version_and_protocol(
    dispatch: &Recorder,
    settings: &RealtimeConnectionSettings,
    litellm_params: &IndexMap<String, Json>,
) {
    let mut named = params();
    named.api_base = Some(String::from("https://named.example"));
    named.api_version = Some(String::from("2025-01-01"));
    named.realtime_protocol = Some(String::from("beta"));
    let call = call(
        "azure_ai/my-deployment",
        &named,
        None,
        litellm_params,
        false,
    );
    let (_, result) = run(dispatch, settings, &call).await;
    let connection = result.unwrap();
    let Branch::Azure {
        api_version,
        protocol,
        ..
    } = &connection.branch
    else {
        panic!("an azure_ai model must take the azure branch");
    };
    assert_eq!(
        connection.args.api_base.as_deref(),
        Some("https://named.example")
    );
    assert_eq!(api_version, "2025-01-01");
    assert_eq!(protocol, "beta");
}

#[tokio::test]
async fn realtime_bedrock_and_xai_arguments() {
    let dispatch = Recorder::default();
    let litellm_params = IndexMap::new();
    bedrock_full_arguments(&dispatch, &litellm_params).await;
    bedrock_ignores_generic_and_global(&dispatch, &litellm_params).await;
    xai_arguments(&dispatch, &litellm_params).await;
    xai_key_fallbacks(&dispatch).await;
    credentials::xai_branch_base_chain_without_a_dynamic_base();
}

fn bedrock_params() -> RealtimeConnectParams {
    let mut connect = params();
    connect.api_base = Some(String::from("https://bedrock.example"));
    connect.api_key = Some(secret("bedrock-key"));
    connect.timeout = Some(30.0_f64);
    connect.aws_region_name = Some(String::from("us-east-1"));
    connect.aws_access_key_id = Some(secret("access"));
    connect.aws_secret_access_key = Some(secret("secret-access"));
    connect.aws_session_token = Some(secret("session"));
    connect.aws_role_name = Some(String::from("role"));
    connect.aws_session_name = Some(String::from("session-name"));
    connect.aws_profile_name = Some(String::from("profile"));
    connect.aws_web_identity_token = Some(secret("web-identity"));
    connect.aws_sts_endpoint = Some(String::from("https://sts.example"));
    connect.aws_bedrock_runtime_endpoint = Some(String::from("https://runtime.example"));
    connect.aws_external_id = Some(String::from("external"));
    connect.aws_session_tags = Some(text("tags"));
    connect
}

async fn bedrock_full_arguments(dispatch: &Recorder, litellm_params: &IndexMap<String, Json>) {
    let connect = bedrock_params();
    let call = call(
        "bedrock/amazon.nova-sonic-v1:0",
        &connect,
        None,
        litellm_params,
        false,
    );
    let (_, result) = run(dispatch, &settings(), &call).await;
    let connection = result.unwrap();
    let Branch::Bedrock(args) = &connection.branch else {
        panic!("a bedrock model must take the bedrock branch");
    };
    assert_eq!(
        connection.args.api_base.as_deref(),
        Some("https://bedrock.example")
    );
    assert_eq!(
        connection
            .args
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("bedrock-key")
    );
    assert_eq!(connection.args.timeout, Some(30.0_f64));
    assert_eq!(connection.args.headers, None);
    assert_eq!(connection.args.litellm_metadata, None);
    assert_eq!(connection.args.user_api_key_dict, None);
    assert!(matches!(
        connection.args.query_params,
        ConnectionQuery::Omitted
    ));
    assert_bedrock_aws_arguments(args);
}

fn assert_bedrock_aws_arguments(args: &RealtimeConnectParams) {
    assert_eq!(args.aws_region_name.as_deref(), Some("us-east-1"));
    assert_eq!(
        args.aws_access_key_id
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("access")
    );
    assert_eq!(
        args.aws_secret_access_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("secret-access")
    );
    assert_eq!(
        args.aws_session_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("session")
    );
    assert_eq!(args.aws_role_name.as_deref(), Some("role"));
    assert_eq!(args.aws_session_name.as_deref(), Some("session-name"));
    assert_eq!(args.aws_profile_name.as_deref(), Some("profile"));
    assert_eq!(
        args.aws_web_identity_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("web-identity")
    );
    assert_eq!(
        args.aws_sts_endpoint.as_deref(),
        Some("https://sts.example")
    );
    assert_eq!(
        args.aws_bedrock_runtime_endpoint.as_deref(),
        Some("https://runtime.example")
    );
    assert_eq!(args.aws_external_id.as_deref(), Some("external"));
    assert_eq!(args.aws_session_tags, Some(text("tags")));
}

async fn bedrock_ignores_generic_and_global(
    dispatch: &Recorder,
    litellm_params: &IndexMap<String, Json>,
) {
    let mut globals = settings();
    globals.api_base = Some(String::from("https://global.example"));
    globals.api_key = Some(secret("sk-global"));
    let mut connect = params();
    connect.generic_api_base = Some(String::from("https://generic.example"));
    let call = call(
        "bedrock/amazon.nova-sonic-v1:0",
        &connect,
        None,
        litellm_params,
        false,
    );
    let (_, result) = run(dispatch, &globals, &call).await;
    let connection = result.unwrap();
    assert_eq!(connection.args.api_base, None);
    assert!(connection.args.api_key.is_none());
}

async fn xai_arguments(dispatch: &Recorder, litellm_params: &IndexMap<String, Json>) {
    let mut xai = settings();
    xai.api_base = Some(String::from("https://global.example"));
    xai.api_key = Some(secret("sk-global"));
    xai.xai_key = Some(secret("xai-key"));
    xai.xai_api_key = Some(secret("xai-env-key"));
    xai_named_arguments(dispatch, &xai, litellm_params).await;
    xai_base_fallbacks(dispatch, litellm_params).await;
}

async fn xai_named_arguments(
    dispatch: &Recorder,
    xai: &RealtimeConnectionSettings,
    litellm_params: &IndexMap<String, Json>,
) {
    let mut connect = params();
    connect.api_base = Some(String::from("https://named.example"));
    connect.generic_api_base = Some(String::from("https://generic.example"));
    let connection = xai_connection(dispatch, xai, &connect, litellm_params).await;
    assert!(matches!(connection.branch, Branch::Xai));
    assert_eq!(
        connection.args.api_base.as_deref(),
        Some("https://named.example")
    );
    assert_eq!(
        connection
            .args
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("xai-key")
    );
}

// get_llm_provider resolves XAI_API_BASE into the dynamic base, so it arrives through the resolution environment
async fn xai_base_fallbacks(dispatch: &Recorder, litellm_params: &IndexMap<String, Json>) {
    let mut environment = BTreeMap::new();
    environment.insert(
        String::from("XAI_API_BASE"),
        String::from("https://xai-env.example"),
    );
    let mut globals = settings();
    globals.api_base = Some(String::from("https://global.example"));
    let bare = params();
    let request = call("xai/grok-4", &bare, None, litellm_params, false);
    let (_, result) = run_with_environment(dispatch, &globals, &request, &environment).await;
    assert_eq!(
        result.unwrap().args.api_base.as_deref(),
        Some("https://xai-env.example")
    );
    let from_default = xai_connection(dispatch, &globals, &params(), litellm_params).await;
    assert_eq!(
        from_default.args.api_base.as_deref(),
        Some("https://api.x.ai/v1")
    );
}

async fn xai_connection(
    dispatch: &Recorder,
    settings: &RealtimeConnectionSettings,
    connect: &RealtimeConnectParams,
    litellm_params: &IndexMap<String, Json>,
) -> RealtimeConnection<TestConfig> {
    let call = call("xai/grok-4", connect, None, litellm_params, false);
    let (_, result) = run(dispatch, settings, &call).await;
    result.unwrap()
}

async fn xai_key_fallbacks(dispatch: &Recorder) {
    let mut generic = settings();
    generic.api_key = Some(secret("sk-global"));
    assert_api_key(dispatch, &generic, "xai/grok-4", &params(), "sk-global").await;
    let mut from_env = settings();
    from_env.xai_api_key = Some(secret("xai-env-key"));
    assert_api_key(dispatch, &from_env, "xai/grok-4", &params(), "xai-env-key").await;
}

#[tokio::test]
async fn realtime_vertex_credentials_and_refresh() {
    let tokens = ScriptedTokens::with(vec![
        Ok((secret("token-a"), String::from("resolved-project"))),
        Ok((secret("token-b"), String::from("ignored-project"))),
    ]);
    let dispatch = Recorder::with_tokens(Arc::clone(&tokens));
    let (settings, connect, named_credentials, query) = vertex_setup();
    let litellm_params = IndexMap::new();
    let call = call(
        "vertex_ai/gemini-live",
        &connect,
        Some(&query),
        &litellm_params,
        false,
    );
    let (_, result) = run(&dispatch, &settings, &call).await;
    let connection = result.unwrap();
    assert_vertex_connection(&settings, &connection, tokens.as_ref(), &named_credentials).await;
    assert_vertex_token_failures(tokens.as_ref()).await;
    vertex_settings_credentials().await;
    vertex_placement_settings(&litellm_params).await;
    vertex_ai_credentials_alias(&litellm_params).await;
    vertex_generic_base(&litellm_params).await;
    credentials::vertex_ai_project_alias_reaches_the_resolver().await;
    credentials::vertex_ai_credentials_alias_beats_the_environment().await;
    credentials::vertex_zero_credential_timeout_expires_without_resolving().await;
    forwarding::vertex_forwards_the_client().await;
}

async fn vertex_generic_base(litellm_params: &IndexMap<String, Json>) {
    let tokens = ScriptedTokens::with(vec![Ok((
        secret("token"),
        String::from("resolved-project"),
    ))]);
    let dispatch = Recorder::with_tokens(tokens);
    let mut connect = params();
    connect.generic_api_base = Some(String::from("https://vertex-generic.example"));
    let model = "vertex_ai/gemini-live";
    let call = call(model, &connect, None, litellm_params, false);
    let (_, result) = run(&dispatch, &settings(), &call).await;
    let connection = result.unwrap();
    assert_eq!(
        connection.args.api_base.as_deref(),
        Some("https://vertex-generic.example")
    );
}

async fn vertex_placement_settings(litellm_params: &IndexMap<String, Json>) {
    let mut globals = settings();
    globals.vertex_project = Some(String::from("global-project"));
    globals.vertexai_project = Some(String::from("env-project"));
    globals.vertex_location = Some(String::from("global-region"));
    globals.vertexai_location = Some(String::from("env-region"));
    let global_placement = vertex_placement_for(&globals, litellm_params).await;
    assert_eq!(global_placement.0.as_deref(), Some("global-region"));
    assert_eq!(
        global_placement.1,
        (None, Some(String::from("global-project")))
    );
    let mut environment_settings = settings();
    environment_settings.vertexai_project = Some(String::from("env-project"));
    environment_settings.vertexai_location = Some(String::from("env-region"));
    let environment_placement = vertex_placement_for(&environment_settings, litellm_params).await;
    assert_eq!(environment_placement.0.as_deref(), Some("env-region"));
    assert_eq!(
        environment_placement.1,
        (None, Some(String::from("env-project")))
    );
}

async fn vertex_placement_for(
    connection_settings: &RealtimeConnectionSettings,
    litellm_params: &IndexMap<String, Json>,
) -> (Option<String>, (Option<Json>, Option<String>)) {
    let tokens = ScriptedTokens::with(vec![Ok((
        secret("token"),
        String::from("resolved-project"),
    ))]);
    let dispatch = Recorder::with_tokens(Arc::clone(&tokens));
    let connect = params();
    let call = call(
        "vertex_ai/gemini-live",
        &connect,
        None,
        litellm_params,
        false,
    );
    let (_, result) = run(&dispatch, connection_settings, &call).await;
    let Branch::Vertex { location, .. } = result.unwrap().branch else {
        panic!("a vertex_ai model must take the vertex branch");
    };
    let recorded = tokens.calls.lock().unwrap_or_else(PoisonError::into_inner)[0].clone();
    (location, recorded)
}

async fn vertex_ai_credentials_alias(litellm_params: &IndexMap<String, Json>) {
    let tokens = ScriptedTokens::with(vec![Ok((
        secret("token"),
        String::from("resolved-project"),
    ))]);
    let dispatch = Recorder::with_tokens(Arc::clone(&tokens));
    let mut connect = params();
    connect.vertex_ai_credentials = Some(text("ai-credentials"));
    let call = call(
        "vertex_ai/gemini-live",
        &connect,
        None,
        litellm_params,
        false,
    );
    let (_, result) = run(&dispatch, &settings(), &call).await;
    assert!(matches!(result.unwrap().branch, Branch::Vertex { .. }));
    assert_eq!(
        *tokens.calls.lock().unwrap_or_else(PoisonError::into_inner),
        vec![(Some(text("ai-credentials")), None)]
    );
}

async fn vertex_settings_credentials() {
    let tokens = ScriptedTokens::with(vec![Ok((
        secret("token-a"),
        String::from("resolved-project"),
    ))]);
    let dispatch = Recorder::with_tokens(Arc::clone(&tokens));
    let mut settings = settings();
    settings.vertexai_credentials = Some(text("env-credentials"));
    let mut connect = params();
    connect.vertex_credentials = Some(Json::Text(String::new()));
    connect.vertex_project = Some(String::from("named-project"));
    let litellm_params = IndexMap::new();
    let call = call(
        "vertex_ai/gemini-live",
        &connect,
        None,
        &litellm_params,
        false,
    );
    let (_, result) = run(&dispatch, &settings, &call).await;
    assert!(matches!(result.unwrap().branch, Branch::Vertex { .. }));
    assert_eq!(
        *tokens.calls.lock().unwrap_or_else(PoisonError::into_inner),
        vec![(
            Some(text("env-credentials")),
            Some(String::from("named-project"))
        )]
    );
}

fn vertex_setup() -> (
    RealtimeConnectionSettings,
    RealtimeConnectParams,
    Json,
    IndexMap<String, String>,
) {
    let mut settings = settings();
    settings.vertex_project = Some(String::from("global-project"));
    settings.vertex_location = Some(String::from("global-region"));
    settings.vertexai_project = Some(String::from("env-project"));
    settings.vertexai_location = Some(String::from("env-region"));
    settings.vertexai_credentials = Some(text("env-credentials"));
    let named_credentials = Json::Object(IndexMap::from([(
        String::from("type"),
        text("service_account"),
    )]));
    let mut connect = params();
    connect.vertex_credentials = Some(named_credentials.clone());
    connect.vertex_project = Some(String::from("named-project"));
    connect.vertex_location = Some(String::new());
    connect.vertex_ai_location = Some(String::from("ai-location"));
    connect.timeout = Some(4.0_f64);
    connect.api_base = Some(String::from("https://vertex-named.example"));
    connect.generic_api_base = Some(String::from("https://vertex-generic.example"));
    let query = IndexMap::from([(String::from("model"), String::from("gemini-live"))]);
    (settings, connect, named_credentials, query)
}

async fn assert_vertex_connection(
    settings: &RealtimeConnectionSettings,
    connection: &RealtimeConnection<TestConfig>,
    tokens: &ScriptedTokens,
    named_credentials: &Json,
) {
    let Branch::Vertex {
        access_token,
        refresh,
        project,
        location,
    } = &connection.branch
    else {
        panic!("a vertex_ai model must take the vertex branch");
    };
    assert_eq!(
        settings.credential_timeout_seconds.to_bits(),
        20.0_f64.to_bits()
    );
    assert_eq!(access_token.expose_secret(), "token-a");
    assert_eq!(project, "resolved-project");
    assert_eq!(location.as_deref(), Some("ai-location"));
    assert_eq!(
        connection.args.api_base.as_deref(),
        Some("https://vertex-named.example")
    );
    assert!(connection.args.api_key.is_none());
    assert_eq!(connection.args.timeout, Some(4.0_f64));
    let ConnectionQuery::Transport(Some(query)) = &connection.args.query_params else {
        panic!("the vertex transport carries the query");
    };
    assert_eq!(query.get("model"), Some(&String::from("gemini-live")));
    assert_eq!(
        *tokens.calls.lock().unwrap_or_else(PoisonError::into_inner),
        vec![(
            Some(named_credentials.clone()),
            Some(String::from("named-project"))
        )]
    );
    assert_vertex_refresh(refresh, tokens).await;
}

async fn assert_vertex_refresh(refresh: &Arc<dyn AccessTokenResolver>, tokens: &ScriptedTokens) {
    let refreshed = refresh.resolve_access_token().await.unwrap();
    assert_eq!(refreshed.expose_secret(), "token-b");
    let calls = tokens
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].1.as_deref(), Some("resolved-project"));
}

async fn assert_vertex_token_failures(tokens: &ScriptedTokens) {
    let timed_out = resolve_vertex_token(tokens, None, None, 0.0_f64)
        .await
        .unwrap_err();
    assert_eq!(
        timed_out.to_string(),
        "Vertex AI realtime: timed out fetching Google OAuth access token after 0.0s; check network egress from the proxy to the OAuth token endpoint (oauth2.googleapis.com)"
    );
    let failures = ScriptedTokens::with(vec![Err(VertexTokenError {
        message: String::from("boom"),
    })]);
    let failed = resolve_vertex_token(failures.as_ref(), None, None, 20.0_f64)
        .await
        .unwrap_err();
    assert_eq!(failed.to_string(), "boom");
}
