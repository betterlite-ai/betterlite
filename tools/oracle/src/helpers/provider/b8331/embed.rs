use std::time::Duration;

use betterlite_prompts::arguments::json::Object;
use betterlite_provider::data_cloud_dispatch::call::EmbeddingCall;
use betterlite_provider::enterprise_ml_dispatch::embed::{
    self as enterprise_embed, EmbeddingContext,
};
use betterlite_provider::enterprise_ml_dispatch::select::EmbeddingConfig;

use super::case::{Case, Operation};
use super::fixture::Fixture;
use super::wire::{Failure, failure};

// the sap and watsonx embedding arms, each through the config LiteLLM's selection returns
pub(super) async fn call(case: &Case, fixture: &Fixture) -> Result<(), Failure> {
    let config = if case.op == Operation::SapEmbedding {
        EmbeddingConfig::Sap
    } else {
        EmbeddingConfig::Watsonx
    };
    let call = EmbeddingCall {
        model: case.model.clone(),
        input: case.embedding_input(),
        optional_params: Object::new(),
        litellm_params: Object::new(),
        api_key: case.key(),
        api_base: case.api_base(),
        headers: None,
        timeout: Duration::from_secs(30),
    };
    let context = EmbeddingContext {
        env: fixture.source(),
        sap_service_key: None,
        transport: &fixture.transport,
        sagemaker: fixture.deps(),
        clock: &fixture.sap_clock,
    };
    enterprise_embed::dispatch(&call, config, &context)
        .await
        .map(drop)
        .map_err(|error| failure(error.status(), &error))
}
