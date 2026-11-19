use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_search_execution::config::Params;
use betterlite_core::api::vector_search_execution::error::SearchError;
use betterlite_core::api::vector_search_execution::openai::OpenAiSearchConfig;
use betterlite_core::api::vector_search_execution::service::{
    SearchCall, SearchDeps, prepare, send,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_openai::credentials::LiteLlmGlobals;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, source};
use super::{Call, Failure, Outcome};

// The production service over the mock's base: the same config sections and pooled client a request would build
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    // The OpenAI provider config over the env's key, exactly as the proxy wires B527
    let provider = OpenAiSearchConfig::new(
        config.sections.openai_credentials.clone(),
        LiteLlmGlobals::default(),
        config.sections.resource_sdk_settings.openai_api_key.clone(),
        None,
    );
    let origin = source(&config, env)?;
    let mut fixture = Fixture::new("vector_store_search")?;
    let params = search_params(api_base);
    let search = SearchCall {
        vector_store_id: &call.vector_store_id,
        query: &call.query,
        optional_params: &call.optional_params,
        litellm_params: &params,
        extra_headers: call.extra_headers.as_ref(),
        extra_body: call.extra_body.as_ref(),
        timeout: call.timeout_seconds.map(Duration::from_secs),
        embedding_executor: None,
    };
    let deps = SearchDeps {
        config: Arc::new(provider),
        source: &origin,
        supplied: None,
    };
    let prepared = fixture
        .prepare(&deps, call.mode.client_kind(), &search)
        .await?;
    let answer = outcome(send(prepared).await)?;
    Ok(SdkResult::VectorSearchTransport {
        value: Box::new(answer),
    })
}

// GenericLiteLLMParams(api_base=api_base): the base is the only litellm param the case sets
fn search_params(api_base: &str) -> Params {
    let mut params = Params::new();
    params.insert("api_base".to_owned(), Json::Text(api_base.to_owned()));
    params
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(
    result: Result<VectorStoreSearchResponse, SearchError>,
) -> Result<Outcome, SdkHttpError> {
    match result {
        // The record serializes to the provider's own keys, so the observed value is the document the handler returned
        Ok(value) => {
            let encoded = serde_json::to_value(&value).map_err(SdkHttpError::Serialize)?;
            Ok(Outcome {
                result: Some(serde_json::from_value(encoded).map_err(SdkHttpError::Serialize)?),
                failure: None,
            })
        }
        Err(SearchError::Provider(error)) => Ok(Outcome {
            result: None,
            failure: Some(Failure {
                status: error.status_code,
                synthesized: error.status_code_is_synthesized,
                message: error.message,
                headers: error.headers,
            }),
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}
