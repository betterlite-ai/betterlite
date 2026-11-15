use std::sync::Arc;

use async_trait::async_trait;
use betterlite_context::agentic_loop_plan::{JsonMap, get_text};
use futures_util::future::join_all;

use crate::call_transformation::outcome::{SearchFailure, SearchResult, search_outcome};
use crate::call_transformation::wire::{Call, Input};
use crate::execution::WebSearchExecution;
use crate::execution::capabilities::ProviderCapabilities;
use crate::execution::rich_input::rich_search_input;
use crate::execution::runner::SearchRunner;
use crate::execution::search::unclassified;
use crate::records::{RichWebSearchInput, SearchOutcome};
use crate::search_tool_access::authorization::SearchToolGrants;
use crate::search_tool_access::hook_kwargs::SearchHookKwargs;
use crate::search_tool_access::search_metadata::SearchMetadataSource;

const EMPTY_SEARCH_TEXT: &str = "No search query provided";

#[async_trait]
pub trait SearchExecutor: Send + Sync {
    async fn execute_search(
        &self,
        query: &str,
        kwargs: &JsonMap,
        rich: Option<RichWebSearchInput>,
    ) -> SearchResult;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSurface {
    Chat,
    Responses,
}

#[must_use]
pub fn create_empty_search_result() -> SearchResult {
    SearchResult::Succeeded {
        text: EMPTY_SEARCH_TEXT.to_owned(),
        response: None,
    }
}

pub async fn search_batch(
    executor: &dyn SearchExecutor,
    tool_calls: &[Call],
    kwargs: &JsonMap,
    surface: SearchSurface,
) -> Vec<SearchOutcome> {
    let searches = search_requests(tool_calls, surface);
    tracing::debug!(
        "WebSearchInterception: Executing {} search(es) in parallel",
        searches.len()
    );
    let futures = searches.into_iter().map(|(query, rich)| async move {
        match query {
            Some(search_query) => executor.execute_search(&search_query, kwargs, rich).await,
            None => create_empty_search_result(),
        }
    });
    join_all(futures)
        .await
        .into_iter()
        .map(search_outcome)
        .collect()
}

fn search_requests(
    tool_calls: &[Call],
    surface: SearchSurface,
) -> Vec<(Option<String>, Option<RichWebSearchInput>)> {
    tool_calls
        .iter()
        .map(|call| {
            let tool_args = tool_args(call, surface);
            let query = tool_args.and_then(query_text);
            let rich = tool_args.and_then(rich_search_input);
            if let Some(text) = &query {
                tracing::debug!("WebSearchInterception: Queuing search for query='{text}'");
            } else {
                tracing::debug!(
                    "WebSearchInterception: Tool call {} has no query",
                    call.id.as_deref().unwrap_or("None")
                );
            }
            (query, rich)
        })
        .collect()
}

fn tool_args(call: &Call, surface: SearchSurface) -> Option<&Input> {
    match (&call.input, surface) {
        (Input::Object(_), _) => Some(&call.input),
        (Input::Other(_), SearchSurface::Chat) => {
            let function = call.function.as_ref()?;
            match &function.arguments {
                Input::Object(_) => Some(&function.arguments),
                Input::Other(_) => None,
            }
        }
        (Input::Other(_), SearchSurface::Responses) => None,
    }
}

fn query_text(input: &Input) -> Option<String> {
    match input {
        Input::Object(fields) => get_text(fields, "query").filter(|query| !query.is_empty()),
        Input::Other(_) => None,
    }
}

fn hook_kwargs(kwargs: &JsonMap) -> Result<SearchHookKwargs, SearchFailure> {
    serde_json::from_value(
        serde_json::to_value(kwargs).map_err(|error| unclassified(error.to_string()))?,
    )
    .map_err(|error| unclassified(error.to_string()))
}

#[async_trait]
impl<Search, Grants, Metadata, Capabilities> SearchExecutor
    for Arc<WebSearchExecution<Search, Grants, Metadata, Capabilities>>
where
    Search: SearchRunner,
    Grants: SearchToolGrants,
    Grants::Team: Send,
    Metadata: SearchMetadataSource,
    Capabilities: ProviderCapabilities,
{
    async fn execute_search(
        &self,
        query: &str,
        kwargs: &JsonMap,
        rich: Option<RichWebSearchInput>,
    ) -> SearchResult {
        let hook_kwargs = match hook_kwargs(kwargs) {
            Ok(hook_kwargs) => hook_kwargs,
            Err(failure) => return SearchResult::Failed(failure),
        };
        match WebSearchExecution::execute_search(self, query, Some(&hook_kwargs), rich.as_ref())
            .await
        {
            Ok((text, response)) => SearchResult::Succeeded { text, response },
            Err(failure) => SearchResult::Failed(failure),
        }
    }
}
