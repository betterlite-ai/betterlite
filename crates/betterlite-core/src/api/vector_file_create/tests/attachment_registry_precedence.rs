use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_error::envelope::WireField;
use betterlite_models::identity::provider_id::ProviderId;

use super::support::{
    Fixture, REQUEST_TIMEOUT, Registry, Spy, Which, attempt, file, json, listing, object, params,
};
use crate::api::vector_store_http_write::service::ClientKind;

const REGISTRY_VALUES: &str = r#"{"api_key":"sk-registry","custom_llm_provider":"azure","api_base":"https://registry.test/v1"}"#;

fn caller() -> Fixture {
    let mut fixture = Fixture::new();
    fixture
        .kwargs
        .insert(String::from("api_key"), json(r#""sk-caller""#));
    fixture
}

#[tokio::test]
async fn attachment_registry_precedence() {
    let registry = Registry::new(object(REGISTRY_VALUES));
    let opened = caller();
    let first_spy = Spy::replying(file());
    let first_seen = attempt(
        Which::Create,
        &first_spy,
        &opened,
        Some(&registry),
        REQUEST_TIMEOUT,
    )
    .await
    .expect("the attachment call succeeds");
    assert_eq!(first_seen.provider, ProviderId::Openai);
    assert_eq!(first_seen.config, VectorStoreFilesConfigTag::OpenAI);
    assert_eq!(first_seen.api_key.as_deref(), Some("sk-registry"));
    assert_eq!(
        first_seen.api_base.as_deref(),
        Some("https://registry.test/v1")
    );
    assert_eq!(first_seen.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(first_seen.fields, params(r#"{"file_id":"file_a"}"#));

    let mut empty = caller();
    empty.provider = Some(String::new());
    let second_spy = Spy::replying(file());
    let second_seen = attempt(
        Which::Create,
        &second_spy,
        &empty,
        Some(&registry),
        REQUEST_TIMEOUT,
    )
    .await
    .expect("the attachment call succeeds");
    assert_eq!(second_seen.provider, ProviderId::Openai);
    assert_eq!(second_seen.mode, ClientKind::Sync);
    assert_eq!(second_seen.api_key.as_deref(), Some("sk-registry"));
}

#[tokio::test]
async fn attachment_explicit_provider_unsupported() {
    let registry = Registry::new(object(REGISTRY_VALUES));
    let mut azure = caller();
    azure.provider = Some(String::from("azure"));
    let create_spy = Spy::replying(file());
    let create_failure = attempt(
        Which::Create,
        &create_spy,
        &azure,
        Some(&registry),
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the attachment call fails");
    assert_eq!(
        create_failure.message,
        "Vector store file create is not supported for azure"
    );
    assert_eq!(create_failure.class, "APIConnectionError");
    assert_eq!(create_failure.status, 500);
    assert_eq!(create_failure.kind, WireField::Null);
    assert_eq!(create_failure.param, WireField::Null);
    assert_eq!(create_failure.code, WireField::Null);
    assert_eq!(create_spy.call_count(), 0);

    let list_spy = Spy::replying(listing());
    let list_failure = attempt(
        Which::List,
        &list_spy,
        &azure,
        Some(&registry),
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the attachment call fails");
    assert_eq!(
        list_failure.message,
        "Vector store file list is not supported for azure"
    );
    assert_eq!(list_failure.class, "APIConnectionError");
    assert_eq!(list_failure.status, 500);
    assert_eq!(list_spy.call_count(), 0);
}

#[tokio::test]
async fn attachment_unknown_provider_fails_conversion() {
    let mut bogus = caller();
    bogus.provider = Some(String::from("bogus"));
    let spy = Spy::replying(file());
    let failure = attempt(Which::Create, &spy, &bogus, None, REQUEST_TIMEOUT)
        .await
        .expect_err("the attachment call fails");
    assert_eq!(failure.message, "'bogus' is not a valid LlmProviders");
    assert_eq!(failure.class, "APIConnectionError");
    assert_eq!(failure.status, 500);
    assert_eq!(failure.kind, WireField::Null);
    assert_eq!(failure.param, WireField::Null);
    assert_eq!(failure.code, WireField::Null);
    assert_eq!(spy.call_count(), 0);
}

#[tokio::test]
async fn attachment_registry_failure_keeps_caller_key() {
    let registry = Registry {
        values: object(REGISTRY_VALUES),
        failing: true,
    };
    let fixture = caller();
    let spy = Spy::replying(file());
    let seen = attempt(
        Which::Create,
        &spy,
        &fixture,
        Some(&registry),
        REQUEST_TIMEOUT,
    )
    .await
    .expect("the attachment call succeeds");
    assert_eq!(seen.provider, ProviderId::Openai);
    assert_eq!(seen.api_key.as_deref(), Some("sk-caller"));
    assert_eq!(seen.api_base, None);
    assert_eq!(spy.call_count(), 1);
}
