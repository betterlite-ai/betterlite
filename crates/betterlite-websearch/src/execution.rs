pub mod capabilities;
pub mod empty_result;
pub mod response;
pub mod rich_input;
pub mod runner;
pub mod search;
pub mod short_circuit;
mod tests;

use betterlite_openai::search::records::SearchResponse;
use betterlite_prompts::message_fields::last_user::get_last_user_message;

use self::capabilities::ProviderCapabilities;
use self::response::SyntheticSearchResponse;
use self::runner::{SearchCall, SearchRunner};
use self::short_circuit::{ShortCircuitRequest, anthropic_native_tool, synthetic_response};
use crate::call_transformation::native::format_search_response;
use crate::call_transformation::outcome::{SearchFailure, SearchResult, search_outcome};
use crate::records::{RichWebSearchInput, SearchOutcome};
use crate::search_tool_access::authorization::{SearchToolAuthorizer, SearchToolGrants};
use crate::search_tool_access::hook_kwargs::{SearchHookKwargs, caller_from_kwargs};
use crate::search_tool_access::parent_correlation::parent_correlation;
use crate::search_tool_access::search_metadata::{
    SearchMetadataSource, build_search_request_metadata,
};
use crate::search_tool_access::tool_selection::{
    SearchToolConfig, SearchToolParams, SearchToolSelector, selected_search_tool_name,
};
use crate::tools::is_web_search_tool;

pub struct WebSearchExecution<Search, Grants, Metadata, Capabilities> {
    pub enabled_providers: Option<Vec<String>>,
    pub search_tool_name: Option<String>,
    pub search_tools: Option<Vec<SearchToolConfig>>,
    pub runner: Search,
    pub authorizer: SearchToolAuthorizer<Grants>,
    pub metadata_source: Metadata,
    pub capabilities: Capabilities,
}

impl<Search, Grants, Metadata, Capabilities>
    WebSearchExecution<Search, Grants, Metadata, Capabilities>
where
    Search: SearchRunner,
    Grants: SearchToolGrants,
    Metadata: SearchMetadataSource,
    Capabilities: ProviderCapabilities,
{
    pub async fn try_short_circuit_search(
        &self,
        request: &ShortCircuitRequest<'_>,
        kwargs: Option<&SearchHookKwargs>,
    ) -> Option<SyntheticSearchResponse> {
        let tools = request.tools?;
        if tools.is_empty() {
            return None;
        }
        let provider = request.custom_llm_provider.unwrap_or_default();
        let enabled = self
            .enabled_providers
            .as_deref()
            .is_none_or(|enabled| enabled.iter().any(|allowed| allowed == provider));
        if !enabled {
            return None;
        }
        if self
            .capabilities
            .handles_web_search_natively(request.model, provider)
        {
            return None;
        }
        if !tools.iter().all(is_web_search_tool) {
            return None;
        }
        let query = get_last_user_message(request.messages)?;
        tracing::debug!(
            "WebSearchInterception: Short-circuit search detected (provider={}, query='{}')",
            provider,
            query
        );
        let native_tool = tools.iter().find(|tool| anthropic_native_tool(tool));
        let outcome = self.short_circuit_search_outcome(&query, kwargs).await;
        Some(synthetic_response(
            request.model,
            &query,
            native_tool,
            &outcome,
        ))
    }

    pub async fn short_circuit_search_outcome(
        &self,
        query: &str,
        kwargs: Option<&SearchHookKwargs>,
    ) -> SearchOutcome {
        match self.execute_search(query, kwargs, None).await {
            Ok((text, response)) => search_outcome(SearchResult::Succeeded { text, response }),
            Err(failure) => search_outcome(SearchResult::Failed(failure)),
        }
    }

    pub async fn execute_search(
        &self,
        query: &str,
        kwargs: Option<&SearchHookKwargs>,
        rich: Option<&RichWebSearchInput>,
    ) -> Result<(String, Option<SearchResponse>), SearchFailure> {
        let search_tool = SearchToolSelector::new(self.search_tool_name.clone())
            .select_from_router(self.search_tools.as_deref());
        let search_tool_name = selected_search_tool_name(search_tool);
        let (litellm_params, selected_provider) = match search_tool {
            Some(tool) => {
                self.authorizer
                    .authorize(tool, kwargs)
                    .await
                    .map_err(|error| search::unclassified(search::deepest_message(&error)))?;
                (tool.litellm_params.clone(), tool.search_provider.clone())
            }
            None => (SearchToolParams::new(), None),
        };
        let provider =
            selected_provider.unwrap_or_else(|| search::DEFAULT_SEARCH_PROVIDER.to_owned());
        let caller = caller_from_kwargs(kwargs);
        let correlation = parent_correlation(kwargs, caller);
        let metadata = caller.map(|search_caller| {
            build_search_request_metadata(
                &self.metadata_source,
                search_caller,
                search_tool_name,
                &correlation,
            )
        });
        let rich_supported = rich.is_some() && self.provider_supports_rich_search(Some(&provider));
        let params = search::search_params(litellm_params, rich, rich_supported, &correlation)?;
        let call = SearchCall {
            query: search::query_argument(query, rich, rich_supported),
            search_provider: &provider,
            params: &params,
            metadata: metadata.as_ref(),
        };
        let response = self.runner.search(call).await?;
        let text = format_search_response(&response)
            .map_err(|error| search::unclassified(error.to_string()))?;
        Ok((text, Some(response)))
    }

    #[must_use]
    pub fn provider_supports_rich_search(&self, search_provider: Option<&str>) -> bool {
        let Some(provider) = search_provider.filter(|name| !name.is_empty()) else {
            return false;
        };
        self.capabilities.supports_rich_search_input(provider)
    }
}
