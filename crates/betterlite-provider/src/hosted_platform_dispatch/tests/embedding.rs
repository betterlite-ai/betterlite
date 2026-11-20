use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use crate::hosted_platform_dispatch::call::EmbeddingCall;
use crate::hosted_platform_dispatch::embedding::dispatch;

use super::support::{Fixture, headers, secret, secret_text};

fn params(source: &str) -> Object {
    serde_json::from_str(source).expect("the fixture params parse")
}

fn embedding_call(fixture: &Fixture) -> EmbeddingCall {
    EmbeddingCall {
        model: fixture.model.clone(),
        input: fixture.input.clone(),
        optional_params: fixture.optional_params.clone(),
        litellm_params: fixture.litellm_params.clone(),
        api_key: fixture.api_key.clone(),
        api_base: fixture.api_base.clone(),
        headers: fixture.headers.clone(),
        timeout: fixture.timeout.unwrap_or_default(),
        aembedding: fixture.aembedding,
    }
}

#[rstest]
#[case::passed_key(
    Some("sk-passed"),
    Some("sk-global"),
    Some("sk-env"),
    Some("sk-passed")
)]
#[case::global_key(None, Some("sk-global"), Some("sk-env"), Some("sk-global"))]
#[case::environment_key(None, None, Some("sk-env"), Some("sk-env"))]
#[case::no_key(None, None, None, None)]
fn fireworks_embedding_resolves_its_key_chain_in_order(
    #[case] passed: Option<&str>,
    #[case] global: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = Fixture::new();
    fixture.api_key = passed.map(secret);
    fixture.settings.api_key = global.map(secret);
    fixture.settings.environment.fireworks_ai_api_key = environment.map(secret);
    let call = embedding_call(&fixture);
    let resolved = dispatch(ProviderId::FireworksAi, &call, fixture.settings_view())
        .expect("fireworks has an arm of its own")
        .expect("the fireworks arm resolves");
    assert_eq!(secret_text(resolved.api_key.as_ref()).as_deref(), expected);
}

#[test]
fn fireworks_embedding_keeps_the_base_and_params_and_drops_the_headers() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("https://fireworks.example.com/v1"));
    fixture.headers = Some(headers(&[("x-scope", "tenant")]));
    fixture.optional_params = params(r#"{"dimensions":256}"#);
    let call = embedding_call(&fixture);
    let resolved = dispatch(ProviderId::FireworksAi, &call, fixture.settings_view())
        .expect("fireworks has an arm of its own")
        .expect("the fireworks arm resolves");
    assert_eq!(resolved.api_base, call.api_base);
    assert_eq!(resolved.optional_params, call.optional_params);
    assert_eq!(resolved.headers, None);
}

#[rstest]
#[case::together(ProviderId::TogetherAi)]
#[case::nvidia_nim(ProviderId::NvidiaNim)]
fn together_and_nvidia_nim_embeddings_stay_in_the_openai_branch(#[case] provider: ProviderId) {
    let fixture = Fixture::new();
    let call = embedding_call(&fixture);
    assert!(dispatch(provider, &call, fixture.settings_view()).is_none());
}
