use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_error::envelope::WireField;
use betterlite_models::identity::provider_id::ProviderId;

use super::super::error::VectorStoreWriteFailure;
use super::support::{
    Fixture, REQUEST_TIMEOUT, Recorded, Resolution, Spy, call_adelete, call_aupdate, call_delete,
    call_update, success,
};

fn provider_fixture(provider: &str) -> Fixture {
    let mut fixture = Fixture::new();
    fixture.provider = Some(provider.to_owned());
    fixture
}

async fn seen_update(spy: &Spy, provider: Option<&str>, resolution: &Resolution) -> Recorded {
    let mut fixture = Fixture::new();
    fixture.provider = provider.map(str::to_owned);
    call_update(spy, &fixture, resolution, REQUEST_TIMEOUT)
        .await
        .expect("the write succeeds");
    spy.seen().expect("the service saw the write")
}

async fn refused_update(
    spy: &Spy,
    provider: &str,
    resolution: &Resolution,
) -> VectorStoreWriteFailure {
    let fixture = provider_fixture(provider);
    call_update(spy, &fixture, resolution, REQUEST_TIMEOUT)
        .await
        .expect_err("the provider is refused before the write")
}

async fn refused_aupdate(
    spy: &Spy,
    provider: &str,
    resolution: &Resolution,
) -> VectorStoreWriteFailure {
    let fixture = provider_fixture(provider);
    call_aupdate(spy, &fixture, resolution, REQUEST_TIMEOUT)
        .await
        .expect_err("the provider is refused before the write")
}

async fn refused_delete(
    spy: &Spy,
    provider: &str,
    resolution: &Resolution,
) -> VectorStoreWriteFailure {
    let fixture = provider_fixture(provider);
    call_delete(spy, &fixture, resolution, REQUEST_TIMEOUT)
        .await
        .expect_err("the provider is refused before the write")
}

async fn refused_adelete(
    spy: &Spy,
    provider: &str,
    resolution: &Resolution,
) -> VectorStoreWriteFailure {
    let fixture = provider_fixture(provider);
    call_adelete(spy, &fixture, resolution, REQUEST_TIMEOUT)
        .await
        .expect_err("the provider is refused before the write")
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
    let absent = seen_update(&spy, None, &resolution).await;
    let named = seen_update(&spy, Some("openai"), &resolution).await;
    assert_eq!(absent, named);
    let qualified = seen_update(&spy, Some("openai/gpt-4o"), &resolution).await;
    assert_eq!(qualified.provider, ProviderId::Openai);
    assert_eq!(qualified.config, VectorStoresConfigTag::OpenAI);
    let refused = Spy::replying(success());
    let unknown = refused_update(&refused, "not_a_provider", &resolution).await;
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
    let update_refusal = refused_update(&spy, "anthropic", &resolution).await;
    assert!(
        update_refusal
            .error
            .to_string()
            .contains("Vector store update is not supported for anthropic")
    );
    assert_connection_failure(&update_refusal);
    let delete_refusal = refused_delete(&spy, "anthropic", &resolution).await;
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
    let update_refusal = refused_aupdate(&spy, "anthropic", &resolution).await;
    assert_connection_failure(&update_refusal);
    let delete_refusal = refused_adelete(&spy, "anthropic", &resolution).await;
    assert_connection_failure(&delete_refusal);
    assert_eq!(spy.call_count(), 0);
}

// get_llm_provider returns the model after the slash as api_type, which B010 reads for vertex_ai
#[tokio::test]
async fn api_type_selects_the_vector_store_config() {
    let resolution = Resolution::new();
    let spy = Spy::replying(success());
    let rag = seen_update(&spy, Some("vertex_ai/rag_api"), &resolution).await;
    assert_eq!(rag.provider, ProviderId::VertexAi);
    assert_eq!(rag.config, VectorStoresConfigTag::Vertex);
    let search = seen_update(&spy, Some("vertex_ai/search_api"), &resolution).await;
    assert_eq!(search.config, VectorStoresConfigTag::VertexSearchAPI);
    let refused = Spy::replying(success());
    let unknown_type = refused_update(&refused, "vertex_ai/embedding", &resolution).await;
    assert!(
        unknown_type
            .error
            .to_string()
            .contains("Vector store update is not supported for vertex_ai")
    );
}
