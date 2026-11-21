#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::identity::provider_id::ProviderId;
use secrecy::SecretString;

use super::error::ContainerOpError;
use super::test_support::{
    CONTAINER_ID, Fixture, INITIAL_CALL_TYPE, StubDecode, StubDecoder, StubLookup, decoding_to,
    params_of,
};

#[test]
fn provider_the_decoder_gets_the_caller_provider_and_the_credentials() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from("sk-test"));
    fixture.api_base = Some("https://api.example/v1".to_owned());
    fixture.api_version = Some("2024-01-01".to_owned());

    fixture.retrieve().expect("the call runs");

    let seen = fixture.decoder.seen();
    let (container_id, provider, params) = seen.first().expect("one decode");
    assert_eq!(container_id, CONTAINER_ID);
    assert_eq!(provider.as_str(), "openai");
    assert_eq!(
        params,
        &params_of(
            r#"{"api_key":"sk-test","api_base":"https://api.example/v1","api_version":"2024-01-01","litellm_call_id":"call-1"}"#
        )
    );
    let calls = fixture.calls();
    assert_eq!(calls.first().expect("one call").params, *params);
}

#[test]
fn provider_the_handler_gets_the_decoded_original_id() {
    let mut fixture = Fixture::new();
    fixture.container_id = "cntr_managed_abc".to_owned();
    fixture.decoder = StubDecoder::answering(decoding_to("cntr_original", "openai", None));

    let object = fixture.retrieve().expect("the call runs");

    assert_eq!(
        fixture.calls().first().expect("one call").container_id,
        "cntr_original"
    );
    assert_eq!(object.id, CONTAINER_ID);
}

#[test]
fn provider_the_handler_gets_the_resolved_providers_config() {
    let mut fixture = Fixture::new();
    fixture.container_id = "cntr_managed_azure".to_owned();
    fixture.decoder = StubDecoder::answering(decoding_to("cntr_original", "azure", None));
    fixture.lookup = StubLookup::carrying("azure-config");

    fixture.retrieve().expect("the call runs");

    assert_eq!(
        fixture
            .calls()
            .first()
            .expect("one call")
            .config_label
            .as_str(),
        "azure-config"
    );
    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Azure]);
}

#[test]
fn provider_an_unknown_name_never_reaches_the_lookup() {
    let mut fixture = Fixture::new();
    fixture.decoder = StubDecoder::answering(StubDecode::Decoded {
        container_id: CONTAINER_ID.to_owned(),
        provider: "not_a_provider".to_owned(),
        model_id: None,
    });

    let failure = fixture.retrieve().expect_err("the provider is unknown");

    let ContainerOpError::UnknownProvider { provider } = &failure.error else {
        panic!("expected an unknown provider, got {:?}", failure.error);
    };
    assert_eq!(provider.as_str(), "not_a_provider");
    assert!(
        failure
            .error
            .to_string()
            .contains("is not a provider betterlite knows")
    );
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected a value error, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("not_a_provider"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.calls().is_empty());
    assert_eq!(fixture.logging.lock().unwrap().call_type, INITIAL_CALL_TYPE);
}

#[test]
fn provider_a_known_provider_without_a_config_reports_the_missing_config() {
    let mut fixture = Fixture::new();
    fixture.decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "anthropic", None));
    fixture.lookup = StubLookup::without_config();

    let failure = fixture.retrieve().expect_err("the lookup answers nothing");

    let ContainerOpError::ProviderConfigMissing { provider } = &failure.error else {
        panic!("expected a missing config, got {:?}", failure.error);
    };
    assert_eq!(provider.as_str(), "anthropic");
    assert_eq!(
        failure.error.to_string(),
        "Container provider config not found for provider: anthropic"
    );
    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Anthropic]);
    assert!(fixture.calls().is_empty());
    assert_eq!(fixture.logging.lock().unwrap().call_type, INITIAL_CALL_TYPE);
}
