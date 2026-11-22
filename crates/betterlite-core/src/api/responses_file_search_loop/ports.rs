use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;

use crate::api::responses_tool_namespaces::values::Fields;

use super::records::SearchAuthorization;

// the aresponses call the emulated flow makes twice (emulated_handler.py:392-L395, L542-L547, L606-L611)
#[async_trait]
pub trait ResponsesFollowUp: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn call(&self, request: FollowUpRequest<'_>)
    -> Result<ResponsesApiResponse, Self::Error>;
}

#[derive(Debug, Clone)]
pub struct FollowUpRequest<'call> {
    pub input: &'call Json,
    pub model: &'call str,
    pub tools: Option<&'call [Json]>,
    pub kwargs: &'call Fields,
    pub internal: bool,
}

// litellm.vector_stores.main.asearch as _run_vector_searches calls it (emulated_handler.py:154-L166)
#[async_trait]
pub trait VectorStoreSearches: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn asearch(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, Self::Error>;
}

#[derive(Debug, Clone)]
pub struct SearchRequest<'search> {
    pub vector_store_id: &'search str,
    pub query: &'search str,
    pub authorization: &'search SearchAuthorization,
}
