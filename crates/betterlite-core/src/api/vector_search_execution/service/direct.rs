use std::time::Duration;

use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;

use super::super::config::{
    DirectSearchConfig, DirectSearchRequest, Params, QueryEmbeddingExecutor, SearchQuery,
    VectorStoreLogging,
};
use super::super::error::SearchError;
use super::logging::log_direct;
use crate::api::vector_store_http_write::service::{ClientKind, LogContext};

pub struct DirectCall<'call> {
    pub provider: &'call str,
    pub vector_store_id: &'call str,
    pub query: &'call SearchQuery,
    pub optional_params: &'call Params,
    pub litellm_params: &'call Params,
    pub timeout: Option<Duration>,
    pub embedding_executor: Option<&'call dyn QueryEmbeddingExecutor>,
}

// The direct handler logs the synthetic endpoint and lets the store run its own search; no HTTP client is selected
pub async fn execute_direct_search<C>(
    config: &C,
    call: &DirectCall<'_>,
    mode: ClientKind,
    log: &mut LogContext<'_, '_>,
) -> Result<VectorStoreSearchResponse, SearchError>
where
    C: DirectSearchConfig + ?Sized,
{
    let mut logging = VectorStoreLogging::default();
    let request = prepare_direct(call, log, &mut logging)?;
    let answer = match mode {
        ClientKind::Sync => config.execute_search_request(request),
        ClientKind::Asynchronous => config.aexecute_search_request(request).await,
    };
    answer.map_err(SearchError::Direct)
}

// log_direct runs first, so a failed pre-call never reaches the store
fn prepare_direct<'call>(
    call: &'call DirectCall<'_>,
    log: &mut LogContext<'_, '_>,
    logging: &'call mut VectorStoreLogging,
) -> Result<DirectSearchRequest<'call>, SearchError> {
    log_direct(call.provider, call.vector_store_id, call.query, log)?;
    Ok(direct_request(call, logging))
}

// dict(litellm_params) is the params copy the store receives; extra_headers and extra_body never reach a direct store
const fn direct_request<'call>(
    call: &'call DirectCall<'_>,
    logging: &'call mut VectorStoreLogging,
) -> DirectSearchRequest<'call> {
    DirectSearchRequest {
        vector_store_id: call.vector_store_id,
        query: call.query,
        optional_params: call.optional_params,
        logging,
        litellm_params: call.litellm_params,
        embedding_executor: call.embedding_executor,
        timeout: call.timeout,
    }
}
