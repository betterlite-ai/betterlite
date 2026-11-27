use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::error::VectorFileWriteFailure;
use super::support::{Fixture, REQUEST_TIMEOUT, Registry, Spy, Which, call, recorded, success};

fn provider_fixture(provider: &str) -> Fixture {
    let mut fixture = Fixture::new();
    fixture.arguments.provider = Some(provider.to_owned());
    fixture
}

// exception_type maps a local ValueError to APIConnectionError with its 500 and no wire fields
fn assert_connection_failure(failure: &VectorFileWriteFailure) {
    let raised = failure
        .raised()
        .expect("APIConnectionError has a projection");
    assert_eq!(raised.class, "APIConnectionError");
    assert_eq!(raised.status, 500_u16);
    assert_eq!(raised.kind, WireField::Null);
    assert_eq!(raised.param, WireField::Null);
    assert_eq!(raised.code, None);
}

#[tokio::test]
async fn file_write_provider_resolution() {
    let spy = Spy::replying(success());
    let absent = recorded(Which::Update, &spy, &Fixture::new(), REQUEST_TIMEOUT).await;
    assert_eq!(absent.provider, ProviderId::Openai);
    assert_eq!(absent.config, VectorStoreFilesConfigTag::OpenAI);
    assert_eq!(absent.provider_name.as_deref(), Some("openai"));
    // _ensure_provider treats the empty name like an absent one
    let empty = recorded(Which::Delete, &spy, &provider_fixture(""), REQUEST_TIMEOUT).await;
    assert_eq!(empty.provider, ProviderId::Openai);
    assert_eq!(empty.config, VectorStoreFilesConfigTag::OpenAI);
    let named = recorded(
        Which::Update,
        &spy,
        &provider_fixture("openai"),
        REQUEST_TIMEOUT,
    )
    .await;
    assert_eq!(named.config, VectorStoreFilesConfigTag::OpenAI);
}

#[tokio::test]
async fn registry_credentials_replace_the_callers() {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.kwargs.insert(
        String::from("api_key"),
        Json::Text(String::from("sk-caller")),
    );
    fixture.registry = Some(Registry::replying(IndexMap::from([
        (
            String::from("api_key"),
            Json::Text(String::from("sk-registry")),
        ),
        (
            String::from("custom_llm_provider"),
            Json::Text(String::from("azure")),
        ),
    ])));
    let seen = recorded(Which::Update, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.api_key.as_deref(), Some("sk-registry"));
    // GenericLiteLLMParams' dump carries the merged key, plus the five fields that default to false
    assert_eq!(
        seen.litellm_params.get("api_key"),
        Some(&Json::Text(String::from("sk-registry")))
    );
    assert_eq!(
        seen.litellm_params.get("use_litellm_proxy"),
        Some(&Json::Bool(false))
    );
    // the provider is resolved before the merge, so the registry's azure lands in the logged params only
    assert_eq!(seen.provider, ProviderId::Openai);
    assert_eq!(seen.provider_name.as_deref(), Some("openai"));
    assert_eq!(
        seen.details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert_eq!(
        fixture.registry.as_ref().map(Registry::lookup_count),
        Some(1)
    );
}

#[tokio::test]
async fn registry_failure_keeps_the_callers_credentials() {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.kwargs.insert(
        String::from("api_key"),
        Json::Text(String::from("sk-caller")),
    );
    fixture.registry = Some(Registry::failing());
    let seen = recorded(Which::Update, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.api_key.as_deref(), Some("sk-caller"));
    assert_eq!(seen.provider, ProviderId::Openai);
    assert_eq!(
        fixture.registry.as_ref().map(Registry::lookup_count),
        Some(1)
    );
}

#[rstest]
#[case::update(Which::Update, "update")]
#[case::delete(Which::Delete, "delete")]
#[tokio::test]
async fn unsupported_providers_fail_before_http(#[case] which: Which, #[case] operation: &str) {
    let spy = Spy::replying(success());
    let failure = call(which, &spy, &provider_fixture("azure"), REQUEST_TIMEOUT)
        .await
        .expect_err("the provider is refused before the write");
    assert_eq!(
        failure.error.to_string(),
        format!("Vector store file {operation} is not supported for azure")
    );
    assert_connection_failure(&failure);
    assert_eq!(spy.call_count(), 0);
}

#[tokio::test]
async fn invalid_providers_fail_enum_conversion() {
    let spy = Spy::replying(success());
    let failure = call(
        Which::Update,
        &spy,
        &provider_fixture("bogus"),
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the provider is not an enum value");
    assert_eq!(
        failure.error.to_string(),
        "'bogus' is not a valid LlmProviders"
    );
    assert_connection_failure(&failure);
    assert_eq!(spy.call_count(), 0);
}
