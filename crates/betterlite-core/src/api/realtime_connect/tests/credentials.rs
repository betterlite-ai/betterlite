use std::sync::{Arc, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::ProviderIdentity;
use indexmap::IndexMap;

use super::super::branches::{DirectProvider, direct_connection};
use super::super::dispatch::Prepared;
use super::super::{Branch, RealtimeConnectError, RealtimeConnection};
use super::assert_api_key;
use super::support::{
    Recorder, ScriptedTokens, TestConfig, call, params, run, secret, settings, text,
};
use crate::api::realtime_contracts::error::HeaderMap;

pub(super) async fn openai_credentials(
    dispatch: &Recorder,
    litellm_params: &IndexMap<String, Json>,
) {
    let mut openai = settings();
    openai.api_key = None;
    openai.openai_key = Some(secret("sk-openai-global"));
    let connect = params();
    let request = call("openai/gpt-realtime", &connect, None, litellm_params, false);
    let (_, result) = run(dispatch, &openai, &request).await;
    let connection = result.unwrap();
    assert!(matches!(connection.branch, Branch::OpenAi));
    assert_eq!(
        connection.args.api_base.as_deref(),
        Some("https://api.openai.com/")
    );
    assert_eq!(
        connection
            .args
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("sk-openai-global")
    );
}

pub(super) async fn openai_settings_key_fallbacks() {
    let dispatch = Recorder::default();
    let mut environment_only = settings();
    environment_only.openai_api_key = Some(secret("openai-only-key"));
    assert_api_key(
        &dispatch,
        &environment_only,
        "openai/gpt-realtime",
        &params(),
        "openai-only-key",
    )
    .await;
}

pub(super) async fn openai_generic_base_and_global_key_win() {
    let dispatch = Recorder::default();
    let mut connection_settings = settings();
    connection_settings.api_base = Some(String::from("https://global.example"));
    connection_settings.api_key = Some(secret("sk-global"));
    connection_settings.openai_key = Some(secret("sk-openai-global"));
    connection_settings.openai_api_key = Some(secret("openai-key"));
    let mut connect = params();
    connect.generic_api_base = Some(String::from("https://generic.example"));
    let litellm_params = IndexMap::new();
    let request = call(
        "openai/gpt-realtime",
        &connect,
        None,
        &litellm_params,
        false,
    );
    let (_, result) = run(&dispatch, &connection_settings, &request).await;
    let connection = result.unwrap();
    assert!(matches!(connection.branch, Branch::OpenAi));
    assert_eq!(
        connection.args.api_base.as_deref(),
        Some("https://generic.example")
    );
    assert_eq!(
        connection
            .args
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("sk-global")
    );
}

// get_llm_provider always fills the xai base, so the generic and environment slots are pinned here
pub(super) fn xai_branch_base_chain_without_a_dynamic_base() {
    let prepared = Prepared {
        model: String::from("grok-4"),
        provider: ProviderIdentity::Known(ProviderId::Xai),
        dynamic_api_base: None,
        dynamic_api_key: None,
        headers: HeaderMap::default(),
        query_params: None,
        metadata: IndexMap::new(),
    };
    let litellm_params = IndexMap::new();
    let mut environment = settings();
    environment.xai_api_base = Some(String::from("https://xai-env.example"));
    let mut generic = params();
    generic.generic_api_base = Some(String::from("https://xai-generic.example"));
    let bare = params();
    let defaults = settings();
    for (connection_settings, connect, expected) in [
        (&environment, &generic, "https://xai-generic.example"),
        (&environment, &bare, "https://xai-env.example"),
        (&defaults, &bare, "https://api.x.ai/v1"),
    ] {
        let request = call("xai/grok-4", connect, None, &litellm_params, false);
        let connection: RealtimeConnection<TestConfig> = direct_connection(
            DirectProvider::Xai,
            connection_settings,
            &request,
            &prepared,
        );
        assert_eq!(connection.args.api_base.as_deref(), Some(expected));
    }
}

pub(super) async fn vertex_ai_project_alias_reaches_the_resolver() {
    let tokens = ScriptedTokens::with(vec![Ok((
        secret("token"),
        String::from("resolved-project"),
    ))]);
    let dispatch = Recorder::with_tokens(Arc::clone(&tokens));
    let mut connect = params();
    connect.vertex_ai_project = Some(String::from("alias-project"));
    let litellm_params = IndexMap::new();
    let request = call(
        "vertex_ai/gemini-live",
        &connect,
        None,
        &litellm_params,
        false,
    );
    let (_, result) = run(&dispatch, &settings(), &request).await;
    assert!(matches!(result.unwrap().branch, Branch::Vertex { .. }));
    assert_eq!(
        *tokens.calls.lock().unwrap_or_else(PoisonError::into_inner),
        vec![(None, Some(String::from("alias-project")))]
    );
}

pub(super) async fn vertex_ai_credentials_alias_beats_the_environment() {
    let tokens = ScriptedTokens::with(vec![Ok((
        secret("token"),
        String::from("resolved-project"),
    ))]);
    let dispatch = Recorder::with_tokens(Arc::clone(&tokens));
    let mut connection_settings = settings();
    connection_settings.vertexai_credentials = Some(text("env-credentials"));
    let mut connect = params();
    connect.vertex_ai_credentials = Some(text("alias-credentials"));
    let litellm_params = IndexMap::new();
    let request = call(
        "vertex_ai/gemini-live",
        &connect,
        None,
        &litellm_params,
        false,
    );
    let (_, result) = run(&dispatch, &connection_settings, &request).await;
    assert!(matches!(result.unwrap().branch, Branch::Vertex { .. }));
    assert_eq!(
        *tokens.calls.lock().unwrap_or_else(PoisonError::into_inner),
        vec![(Some(text("alias-credentials")), None)]
    );
}

pub(super) async fn vertex_zero_credential_timeout_expires_without_resolving() {
    let tokens = ScriptedTokens::with(Vec::new());
    let dispatch = Recorder::with_tokens(Arc::clone(&tokens));
    let mut connection_settings = settings();
    connection_settings.credential_timeout_seconds = 0.0_f64;
    let connect = params();
    let litellm_params = IndexMap::new();
    let request = call(
        "vertex_ai/gemini-live",
        &connect,
        None,
        &litellm_params,
        false,
    );
    let (_, result) = run(&dispatch, &connection_settings, &request).await;
    let Err(error) = result else {
        panic!("a zero credential timeout must expire before the resolver runs");
    };
    assert!(matches!(
        &error,
        RealtimeConnectError::VertexTokenTimeout { .. }
    ));
    assert_eq!(
        error.to_string(),
        "Vertex AI realtime: timed out fetching Google OAuth access token after 0.0s; check network egress from the proxy to the OAuth token endpoint (oauth2.googleapis.com)"
    );
    assert!(
        tokens
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}
