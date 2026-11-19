use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Object;

use crate::hosted_platform_dispatch::embedding::dispatch;

use super::support::{Fixture, headers, secret, secret_text};

fn params(source: &str) -> Object {
    serde_json::from_str(source).expect("the fixture params parse")
}

#[test]
fn fireworks_embedding_resolves_its_key_chain_in_order() {
    let cases = [
        (
            Some("sk-passed"),
            Some("sk-global"),
            Some("sk-env"),
            Some("sk-passed"),
        ),
        (None, Some("sk-global"), Some("sk-env"), Some("sk-global")),
        (None, None, Some("sk-env"), Some("sk-env")),
        (None, None, None, None),
    ];
    for (passed, global, environment, expected) in cases {
        let mut fixture = Fixture::new();
        fixture.api_key = passed.map(secret);
        fixture.settings.api_key = global.map(secret);
        fixture.settings.environment.fireworks_ai_api_key = environment.map(secret);
        let resolved = dispatch(
            ProviderId::FireworksAi,
            &fixture.embedding_call(),
            fixture.settings_view(),
        )
        .expect("fireworks has an arm of its own")
        .expect("the fireworks arm resolves");
        assert_eq!(secret_text(resolved.api_key.as_ref()).as_deref(), expected);
    }
}

#[test]
fn fireworks_embedding_keeps_the_base_and_params_and_drops_the_headers() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("https://fireworks.example.com/v1"));
    fixture.headers = Some(headers(&[("x-scope", "tenant")]));
    fixture.optional_params = params(r#"{"dimensions":256}"#);
    let call = fixture.embedding_call();
    let resolved = dispatch(ProviderId::FireworksAi, &call, fixture.settings_view())
        .expect("fireworks has an arm of its own")
        .expect("the fireworks arm resolves");
    assert_eq!(resolved.api_base, call.api_base);
    assert_eq!(resolved.optional_params, call.optional_params);
    assert_eq!(resolved.headers, None);
}

#[test]
fn together_and_nvidia_nim_embeddings_stay_in_the_openai_branch() {
    let fixture = Fixture::new();
    let call = fixture.embedding_call();
    for provider in [ProviderId::TogetherAi, ProviderId::NvidiaNim] {
        assert!(dispatch(provider, &call, fixture.settings_view()).is_none());
    }
}
