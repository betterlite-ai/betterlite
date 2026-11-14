use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_openai::search::provider::SearchQuery;
use betterlite_openai::search::records::SearchResponse;
use serde_json::json;

use crate::call_transformation::outcome::{SearchFailure, SearchResult};
use crate::chat_responses_followup::search::SearchExecutor;
use crate::execution::WebSearchExecution;
use crate::execution::capabilities::ProviderCapabilities;
use crate::execution::runner::{SearchCall, SearchRunner};
use crate::execution::search::unclassified;
use crate::records::RichWebSearchInput;
use crate::search_tool_access::authorization::{SearchToolAuthorizer, SearchToolGrants};
use crate::search_tool_access::hook_kwargs::{ParentOtelSpan, SearchCaller};
use crate::search_tool_access::search_metadata::{
    SearchMetadataSource, SearchMetadataValue, SearchRequestMetadata,
};
use crate::search_tool_access::tool_selection::SearchToolParams;

struct RecordedCall {
    query: SearchQuery,
    params: SearchToolParams,
    search_provider: String,
    metadata: Option<SearchRequestMetadata>,
}

struct FakeRunner {
    outcome: Result<SearchResponse, SearchFailure>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl SearchRunner for FakeRunner {
    fn search(
        &self,
        call: SearchCall<'_>,
    ) -> impl Future<Output = Result<SearchResponse, SearchFailure>> + Send {
        self.calls.lock().unwrap().push(RecordedCall {
            query: call.query,
            params: call.params.clone(),
            search_provider: call.search_provider.to_owned(),
            metadata: call.metadata.cloned(),
        });
        let outcome = self.outcome.clone();
        async move { outcome }
    }
}

struct FakeGrants;

impl SearchToolGrants for FakeGrants {
    type Team = ();
    type Error = Infallible;

    fn check_key(
        &self,
        _name: &str,
        _caller: &SearchCaller,
    ) -> impl Future<Output = Result<(), Infallible>> + Send {
        ready(Ok(()))
    }

    fn load_team(
        &self,
        _team_id: &str,
        _parent_otel_span: Option<&ParentOtelSpan>,
    ) -> impl Future<Output = Result<Self::Team, Infallible>> + Send {
        ready(Ok(()))
    }

    fn check_team(
        &self,
        _name: &str,
        _team: &Self::Team,
    ) -> impl Future<Output = Result<(), Infallible>> + Send {
        ready(Ok(()))
    }
}

struct FakeMetadata;

impl SearchMetadataSource for FakeMetadata {
    fn sanitized_key_metadata(&self, _caller: &SearchCaller) -> SearchRequestMetadata {
        let mut entries = SearchRequestMetadata::new();
        entries.insert(
            "sanitized".to_owned(),
            SearchMetadataValue::Text("yes".to_owned()),
        );
        entries
    }

    fn logged_api_key(&self, _caller: &SearchCaller) -> Option<String> {
        Some("sk-test".to_owned())
    }
}

struct FakeCapabilities {
    rich: bool,
}

impl ProviderCapabilities for FakeCapabilities {
    fn handles_web_search_natively(&self, _model: &str, _provider: &str) -> bool {
        false
    }

    fn supports_rich_search_input(&self, _search_provider: &str) -> bool {
        self.rich
    }
}

fn search_response() -> SearchResponse {
    serde_json::from_str(r#"{"results":[{"title":"T","url":"https://example.com","snippet":"S"}]}"#)
        .unwrap()
}

fn execution() -> WebSearchExecution<FakeRunner, FakeGrants, FakeMetadata, FakeCapabilities> {
    WebSearchExecution {
        enabled_providers: None,
        search_tool_name: None,
        search_tools: None,
        runner: FakeRunner {
            outcome: Ok(search_response()),
            calls: Mutex::new(Vec::new()),
        },
        authorizer: SearchToolAuthorizer {
            grants_port: FakeGrants,
        },
        metadata_source: FakeMetadata,
        capabilities: FakeCapabilities { rich: false },
    }
}

fn kwargs(value: serde_json::Value) -> JsonMap {
    serde_json::from_str(&value.to_string()).unwrap()
}

#[tokio::test]
async fn websearch_execution_port_forwards_the_caller_kwargs_to_the_search() {
    let execution = Arc::new(execution());
    let kwargs = kwargs(json!({"metadata": {"user_api_key_auth": {"team_id": "team_1"}}}));
    let result = SearchExecutor::execute_search(&execution, "weather", &kwargs, None).await;
    assert!(matches!(result, SearchResult::Succeeded { .. }));
    let calls = execution.runner.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].search_provider, "perplexity");
    let metadata = calls[0].metadata.as_ref().expect("the caller metadata");
    assert!(matches!(
        metadata.get("sanitized"),
        Some(SearchMetadataValue::Text(text)) if text.as_str() == "yes"
    ));
    assert!(metadata.contains_key("user_api_key_auth"));
    drop(calls);
}

#[tokio::test]
async fn websearch_execution_port_reports_a_failed_search() {
    let mut execution = execution();
    execution.runner.outcome = Err(unclassified("provider down".to_owned()));
    let execution = Arc::new(execution);
    let result =
        SearchExecutor::execute_search(&execution, "weather", &kwargs(json!({})), None).await;
    assert!(matches!(result, SearchResult::Failed(failure) if failure.message == "provider down"));
}

#[tokio::test]
async fn websearch_execution_port_forwards_the_rich_input_to_a_rich_provider() {
    let mut execution = execution();
    execution.capabilities.rich = true;
    let execution = Arc::new(execution);
    let rich = RichWebSearchInput {
        objective: Some("find the forecast".to_owned()),
        search_queries: Some(vec!["weather".to_owned(), "forecast".to_owned()]),
    };
    let result =
        SearchExecutor::execute_search(&execution, "weather", &kwargs(json!({})), Some(rich)).await;
    assert!(matches!(result, SearchResult::Succeeded { .. }));
    let calls = execution.runner.calls.lock().unwrap();
    assert_eq!(
        calls[0].query,
        SearchQuery::Multiple(vec!["weather".to_owned(), "forecast".to_owned()])
    );
    assert_eq!(calls[0].params["objective"].get(), "\"find the forecast\"");
}

#[tokio::test]
async fn websearch_execution_port_keeps_the_single_query_for_a_plain_provider() {
    let execution = Arc::new(execution());
    let rich = RichWebSearchInput {
        objective: Some("find the forecast".to_owned()),
        search_queries: Some(vec!["weather".to_owned(), "forecast".to_owned()]),
    };
    let result =
        SearchExecutor::execute_search(&execution, "weather", &kwargs(json!({})), Some(rich)).await;
    assert!(matches!(result, SearchResult::Succeeded { .. }));
    let calls = execution.runner.calls.lock().unwrap();
    assert_eq!(calls[0].query, SearchQuery::Single("weather".to_owned()));
    assert!(!calls[0].params.contains_key("objective"));
    drop(calls);
}
