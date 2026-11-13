use std::future::Future;

use betterlite_openai::search::provider::SearchQuery;
use betterlite_openai::search::records::SearchResponse;

use crate::call_transformation::outcome::SearchFailure;
use crate::search_tool_access::search_metadata::SearchRequestMetadata;
use crate::search_tool_access::tool_selection::SearchToolParams;

#[derive(Debug)]
pub struct SearchCall<'call> {
    pub query: SearchQuery,
    pub search_provider: &'call str,
    pub params: &'call SearchToolParams,
    pub metadata: Option<&'call SearchRequestMetadata>,
}

pub trait SearchRunner: Send + Sync {
    fn search(
        &self,
        call: SearchCall<'_>,
    ) -> impl Future<Output = Result<SearchResponse, SearchFailure>> + Send;
}
