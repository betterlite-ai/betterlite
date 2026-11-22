use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::embedding::EmbeddingInput;
use rstest::rstest;

use crate::gateway_provider_dispatch::call::EmbeddingCall;
use crate::gateway_provider_dispatch::chat::OPENROUTER_DEFAULT_API_BASE;
use crate::gateway_provider_dispatch::context::DispatchSettings;
use crate::gateway_provider_dispatch::embedding::dispatch;

use super::support::{Fixture, object, secret, secret_text};

fn call() -> EmbeddingCall {
    EmbeddingCall {
        model: String::from("text-embedding-3-small"),
        input: EmbeddingInput::Text(String::from("hello")),
        optional_params: object(r#"{"dimensions": 8}"#),
        litellm_params: object(r#"{"custom_llm_provider": "openrouter"}"#),
        api_key: None,
        api_base: None,
        headers: None,
        timeout: Duration::from_secs(60),
        aembedding: false,
    }
}

fn resolve(provider: ProviderId, call: &EmbeddingCall, fixture: &Fixture) -> EmbeddingCall {
    dispatch(provider, call, DispatchSettings::from(&fixture.settings))
        .expect("the embedding table has this provider")
}

fn resolved_key(
    provider: ProviderId,
    request: &EmbeddingCall,
    fixture: &Fixture,
) -> Option<String> {
    secret_text(resolve(provider, request, fixture).api_key.as_ref())
}

#[rstest]
#[case::openai(ProviderId::Openai)]
#[case::together(ProviderId::TogetherAi)]
#[case::eden(ProviderId::Edenai)]
#[case::cohere(ProviderId::Cohere)]
fn dispatch_leaves_other_embedding_providers_alone(#[case] provider: ProviderId) {
    let fixture = Fixture::new();
    assert!(dispatch(provider, &call(), DispatchSettings::from(&fixture.settings)).is_none());
}

#[test]
fn openrouter_embedding_resolves_the_chat_base_and_key_chains() {
    let mut fixture = Fixture::new();
    fixture.settings.api_base = Some(String::from("http://global"));
    fixture.settings.environment.openrouter_api_base = Some(String::from("http://env"));
    fixture.settings.environment.openrouter_api_key = Some(secret("sk-env"));
    let mut request = call();
    request.api_base = Some(String::from("http://passed"));
    request.api_key = Some(secret("sk-passed"));

    let resolved = resolve(ProviderId::Openrouter, &request, &fixture);
    assert_eq!(resolved.api_base.as_deref(), Some("http://passed"));
    assert_eq!(
        secret_text(resolved.api_key.as_ref()),
        Some(String::from("sk-passed"))
    );

    request.api_base = Some(String::new());
    request.api_key = Some(secret(""));
    let refreshed = resolve(ProviderId::Openrouter, &request, &fixture);
    assert_eq!(refreshed.api_base.as_deref(), Some("http://global"));
    assert_eq!(
        secret_text(refreshed.api_key.as_ref()),
        Some(String::from("sk-env")),
        "the global key is absent, so OPENROUTER_API_KEY answers"
    );
}

#[test]
fn openrouter_embedding_key_falls_through_the_openrouter_and_or_values() {
    let mut fixture = Fixture::new();
    fixture.settings.api_key = Some(secret(""));
    fixture.settings.openrouter_key = Some(secret("sk-litellm"));
    fixture.settings.environment.openrouter_api_key = Some(secret("sk-env"));
    fixture.settings.environment.or_api_key = Some(secret("sk-or"));
    assert_eq!(
        resolved_key(ProviderId::Openrouter, &call(), &fixture),
        Some(String::from("sk-litellm"))
    );

    fixture.settings.openrouter_key = None;
    assert_eq!(
        resolved_key(ProviderId::Openrouter, &call(), &fixture),
        Some(String::from("sk-env"))
    );

    fixture.settings.environment.openrouter_api_key = None;
    assert_eq!(
        resolved_key(ProviderId::Openrouter, &call(), &fixture),
        Some(String::from("sk-or"))
    );

    fixture.settings.environment.or_api_key = None;
    assert_eq!(
        resolved_key(ProviderId::Openrouter, &call(), &fixture),
        None
    );
}

#[test]
fn openrouter_embedding_defaults_the_base_and_adds_the_referer_headers() {
    let fixture = Fixture::new();
    let resolved = resolve(ProviderId::Openrouter, &call(), &fixture);
    assert_eq!(
        resolved.api_base.as_deref(),
        Some(OPENROUTER_DEFAULT_API_BASE)
    );
    assert_eq!(
        resolved.headers,
        Some(BTreeMap::from([
            (
                String::from("HTTP-Referer"),
                String::from("https://litellm.ai")
            ),
            (String::from("X-Title"), String::from("liteLLM")),
        ]))
    );
}

#[test]
fn openrouter_embedding_headers_read_the_environment_and_the_request_wins() {
    let mut fixture = Fixture::new();
    fixture.settings.environment.or_site_url = Some(String::from("https://or.example"));
    fixture.settings.environment.or_app_name = Some(String::from("or-app"));
    let mut request = call();
    request.headers = Some(BTreeMap::from([(
        String::from("X-Title"),
        String::from("caller"),
    )]));
    let resolved = resolve(ProviderId::Openrouter, &request, &fixture);
    assert_eq!(
        resolved.headers,
        Some(BTreeMap::from([
            (
                String::from("HTTP-Referer"),
                String::from("https://or.example")
            ),
            (String::from("X-Title"), String::from("caller")),
        ]))
    );
}

#[test]
fn vercel_embedding_resolves_the_base_and_reads_the_oidc_token_last() {
    let mut fixture = Fixture::new();
    fixture.settings.environment.vercel_ai_gateway_api_base = Some(String::from("http://env"));
    fixture.settings.environment.vercel_ai_gateway_api_key = Some(secret("sk-vercel"));
    fixture.settings.environment.vercel_oidc_token = Some(secret("oidc"));
    let mut request = call();
    request.api_base = Some(String::from("http://passed"));

    let resolved = resolve(ProviderId::VercelAiGateway, &request, &fixture);
    assert_eq!(resolved.api_base.as_deref(), Some("http://passed"));
    assert_eq!(
        secret_text(resolved.api_key.as_ref()),
        Some(String::from("sk-vercel"))
    );

    fixture.settings.environment.vercel_ai_gateway_api_key = Some(secret(""));
    assert_eq!(
        resolved_key(ProviderId::VercelAiGateway, &request, &fixture),
        Some(String::from("oidc")),
        "an empty gateway key falls through to VERCEL_OIDC_TOKEN"
    );

    fixture.settings.environment.vercel_oidc_token = None;
    assert_eq!(
        resolved_key(ProviderId::VercelAiGateway, &request, &fixture),
        None
    );

    request.api_base = None;
    assert_eq!(
        resolve(ProviderId::VercelAiGateway, &request, &fixture)
            .api_base
            .as_deref(),
        Some("http://env")
    );
}

#[test]
fn vercel_embedding_defaults_the_base_and_sends_the_request_headers_alone() {
    let fixture = Fixture::new();
    let resolved = resolve(ProviderId::VercelAiGateway, &call(), &fixture);
    assert_eq!(
        resolved.api_base.as_deref(),
        Some("https://ai-gateway.vercel.sh/v1")
    );
    assert_eq!(
        resolved.headers, None,
        "no referer pair and no global overlay"
    );

    let mut fixture_with_globals = Fixture::new();
    fixture_with_globals.settings.headers = Some(BTreeMap::from([(
        String::from("x-global"),
        String::from("one"),
    )]));
    let mut request = call();
    request.headers = Some(BTreeMap::from([(
        String::from("x-scope"),
        String::from("tenant"),
    )]));
    let layered = resolve(ProviderId::VercelAiGateway, &request, &fixture_with_globals);
    assert_eq!(
        layered.headers,
        Some(BTreeMap::from([(
            String::from("x-scope"),
            String::from("tenant"),
        )])),
        "the request headers pass through and litellm.headers is not laid on"
    );
}

#[test]
fn the_embedding_arms_hand_the_request_fields_through_unchanged() {
    let fixture = Fixture::new();
    let request = call();
    let resolved = resolve(ProviderId::VercelAiGateway, &request, &fixture);
    assert_eq!(resolved.model, "text-embedding-3-small");
    assert_eq!(resolved.input, EmbeddingInput::Text(String::from("hello")));
    assert_eq!(resolved.optional_params, request.optional_params);
    assert_eq!(resolved.litellm_params, request.litellm_params);
    assert_eq!(resolved.timeout, Duration::from_secs(60));
    assert!(!resolved.aembedding);
}
