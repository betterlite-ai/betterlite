use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_error::envelope::WireField;
use betterlite_models::identity::provider_id::ProviderId;

use super::super::error::VectorStoreWriteFailure;
use super::support::{
    Fixture, REQUEST_TIMEOUT, Recorded, Resolution, Spy, Which, call, recorded, success,
};

fn provider_fixture(provider: &str) -> Fixture {
    let mut fixture = Fixture::new();
    fixture.arguments.provider = Some(provider.to_owned());
    fixture
}

async fn recorded_update(provider: Option<&str>, spy: &Spy, resolution: &Resolution) -> Recorded {
    let mut fixture = Fixture::new();
    fixture.arguments.provider = provider.map(String::from);
    recorded(Which::Update, spy, &fixture, resolution, REQUEST_TIMEOUT).await
}

// exception_type maps a local ValueError to APIConnectionError with its 500 and no wire fields
fn assert_connection_failure(failure: &VectorStoreWriteFailure) {
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
async fn write_provider_selection() {
    let resolution = Resolution::new();
    let spy = Spy::replying(success());
    let absent = recorded_update(None, &spy, &resolution).await;
    let named = recorded_update(Some("openai"), &spy, &resolution).await;
    assert_eq!(absent, named);
    let qualified = recorded_update(Some("openai/gpt-4o"), &spy, &resolution).await;
    assert_eq!(qualified.provider, ProviderId::Openai);
    assert_eq!(qualified.config, VectorStoresConfigTag::OpenAI);
    let refused = Spy::replying(success());
    let unknown = call(
        Which::Update,
        &refused,
        &provider_fixture("not_a_provider"),
        &resolution,
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the provider is refused before the write");
    assert!(
        unknown
            .error
            .to_string()
            .contains("'not_a_provider' is not a valid LlmProviders")
    );
    assert_connection_failure(&unknown);
    assert_eq!(refused.call_count(), 0);
}

#[tokio::test]
async fn unsupported_provider_reports_the_operation() {
    let resolution = Resolution::new();
    let spy = Spy::replying(success());
    let update_refusal = call(
        Which::Update,
        &spy,
        &provider_fixture("anthropic"),
        &resolution,
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the provider is refused before the write");
    assert!(
        update_refusal
            .error
            .to_string()
            .contains("Vector store update is not supported for anthropic")
    );
    assert_connection_failure(&update_refusal);
    let delete_refusal = call(
        Which::Delete,
        &spy,
        &provider_fixture("anthropic"),
        &resolution,
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the provider is refused before the write");
    assert!(
        delete_refusal
            .error
            .to_string()
            .contains("Vector store delete is not supported for anthropic")
    );
    assert_connection_failure(&delete_refusal);
    assert_eq!(spy.call_count(), 0);
}

#[tokio::test]
async fn async_refusals_are_mapped_too() {
    let resolution = Resolution::new();
    let spy = Spy::replying(success());
    let update_refusal = call(
        Which::Aupdate,
        &spy,
        &provider_fixture("anthropic"),
        &resolution,
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the provider is refused before the write");
    assert_connection_failure(&update_refusal);
    let delete_refusal = call(
        Which::Adelete,
        &spy,
        &provider_fixture("anthropic"),
        &resolution,
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the provider is refused before the write");
    assert_connection_failure(&delete_refusal);
    assert_eq!(spy.call_count(), 0);
}

// get_llm_provider returns the model after the slash as api_type, which B010 reads for vertex_ai
#[tokio::test]
async fn api_type_selects_the_vector_store_config() {
    let resolution = Resolution::new();
    let spy = Spy::replying(success());
    let rag = recorded(
        Which::Update,
        &spy,
        &provider_fixture("vertex_ai/rag_api"),
        &resolution,
        REQUEST_TIMEOUT,
    )
    .await;
    assert_eq!(rag.provider, ProviderId::VertexAi);
    assert_eq!(rag.config, VectorStoresConfigTag::Vertex);
    let search = recorded(
        Which::Update,
        &spy,
        &provider_fixture("vertex_ai/search_api"),
        &resolution,
        REQUEST_TIMEOUT,
    )
    .await;
    assert_eq!(search.config, VectorStoresConfigTag::VertexSearchAPI);
    let refused = Spy::replying(success());
    let unknown_type = call(
        Which::Update,
        &refused,
        &provider_fixture("vertex_ai/embedding"),
        &resolution,
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the provider is refused before the write");
    assert!(
        unknown_type
            .error
            .to_string()
            .contains("Vector store update is not supported for vertex_ai")
    );
}
