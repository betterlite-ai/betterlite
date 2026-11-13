use std::future::{Future, ready};
use std::mem::take;
use std::sync::Mutex;

use betterlite_error::error::ErrorClass;
use betterlite_openai::search::provider::SearchQuery;
use betterlite_openai::search::records::SearchResponse;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::{Content, Message};
use serde_json::value::RawValue;

use crate::call_transformation::native::{
    ResultBlock, ResultContent, ResultError, ResultItem, search_error_code,
};
use crate::call_transformation::outcome::SearchFailure;
use crate::call_transformation::wire::Input;
use crate::execution::WebSearchExecution;
use crate::execution::capabilities::ProviderCapabilities;
use crate::execution::empty_result::create_empty_search_result;
use crate::execution::response::{
    AssistantRole, MessageKind, ServerToolUseBlock, StopReason, SyntheticContentBlock,
    SyntheticSearchResponse, TextBlock, TextKind,
};
use crate::execution::rich_input::rich_search_input;
use crate::execution::runner::{SearchCall, SearchRunner};
use crate::execution::search::{DEFAULT_SEARCH_PROVIDER, unclassified};
use crate::execution::short_circuit::ShortCircuitRequest;
use crate::records::{RichWebSearchInput, SearchOutcome, WebSearchToolResultErrorCode};
use crate::search_tool_access::authorization::{SearchToolAuthorizer, SearchToolGrants};
use crate::search_tool_access::hook_kwargs::{ParentOtelSpan, SearchCaller, SearchHookKwargs};
use crate::search_tool_access::search_metadata::{
    SearchMetadataSource, SearchMetadataValue, SearchRequestMetadata,
};
use crate::search_tool_access::tool_selection::{SearchToolConfig, SearchToolParams};
use crate::tools::Tool;

struct RecordedCall {
    query: SearchQuery,
    search_provider: String,
    params: SearchToolParams,
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
            search_provider: call.search_provider.to_owned(),
            params: call.params.clone(),
            metadata: call.metadata.cloned(),
        });
        let outcome = self.outcome.clone();
        async move { outcome }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("search tool access denied")]
struct GrantDenied;

struct FakeGrants {
    denied: bool,
}

impl SearchToolGrants for FakeGrants {
    type Team = ();
    type Error = GrantDenied;

    fn check_key(
        &self,
        _name: &str,
        _caller: &SearchCaller,
    ) -> impl Future<Output = Result<(), GrantDenied>> + Send {
        let outcome = if self.denied {
            Err(GrantDenied)
        } else {
            Ok(())
        };
        ready(outcome)
    }

    fn load_team(
        &self,
        _team_id: &str,
        _parent_otel_span: Option<&ParentOtelSpan>,
    ) -> impl Future<Output = Result<(), GrantDenied>> + Send {
        ready(Ok(()))
    }

    fn check_team(
        &self,
        _name: &str,
        _team: &(),
    ) -> impl Future<Output = Result<(), GrantDenied>> + Send {
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
    native_providers: Vec<String>,
    rich_providers: Vec<String>,
}

impl ProviderCapabilities for FakeCapabilities {
    fn handles_web_search_natively(&self, _model: &str, provider: &str) -> bool {
        self.native_providers.iter().any(|name| name == provider)
    }

    fn supports_rich_search_input(&self, search_provider: &str) -> bool {
        self.rich_providers
            .iter()
            .any(|name| name == search_provider)
    }
}

fn runner(response: SearchResponse) -> FakeRunner {
    FakeRunner {
        outcome: Ok(response),
        calls: Mutex::new(Vec::new()),
    }
}

fn failing_runner(failure: SearchFailure) -> FakeRunner {
    FakeRunner {
        outcome: Err(failure),
        calls: Mutex::new(Vec::new()),
    }
}

fn execution(
    runner: FakeRunner,
    capabilities: FakeCapabilities,
) -> WebSearchExecution<FakeRunner, FakeGrants, FakeMetadata, FakeCapabilities> {
    WebSearchExecution {
        enabled_providers: None,
        search_tool_name: None,
        search_tools: None,
        runner,
        authorizer: SearchToolAuthorizer {
            grants_port: FakeGrants { denied: false },
        },
        metadata_source: FakeMetadata,
        capabilities,
    }
}

fn denied_execution(
    search_tools: Vec<SearchToolConfig>,
) -> WebSearchExecution<FakeRunner, FakeGrants, FakeMetadata, FakeCapabilities> {
    WebSearchExecution {
        enabled_providers: None,
        search_tool_name: None,
        search_tools: Some(search_tools),
        runner: runner(search_response()),
        authorizer: SearchToolAuthorizer {
            grants_port: FakeGrants { denied: true },
        },
        metadata_source: FakeMetadata,
        capabilities: empty_capabilities(),
    }
}

fn search_response() -> SearchResponse {
    serde_json::from_str(r#"{"results":[{"title":"T","url":"https://example.com","snippet":"S"}]}"#)
        .unwrap()
}

fn empty_capabilities() -> FakeCapabilities {
    FakeCapabilities {
        native_providers: Vec::new(),
        rich_providers: Vec::new(),
    }
}

fn search_tool(name: &str, provider: &str, extra_params: &str) -> SearchToolConfig {
    let source = format!(
        r#"{{"search_tool_name":"{name}","litellm_params":{{"search_provider":"{provider}"{extra_params}}}}}"#
    );
    serde_json::from_str(&source).unwrap()
}

fn tool(source: &str) -> Tool {
    serde_json::from_str(source).unwrap()
}

fn native_tool() -> Tool {
    tool(r#"{"type":"web_search_20250305","name":"web_search"}"#)
}

fn litellm_tool() -> Tool {
    tool(r#"{"name":"litellm_web_search"}"#)
}

fn user_message(content: &str) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn rate_limit() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"rate_limit","context":{"message":"slow down","model":null,"provider":null},"category":"vendor_rate_limit"}"#,
    )
    .unwrap()
}

fn bad_request() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"bad_request","message":"bad input","model":null,"provider":null}"#,
    )
    .unwrap()
}

fn unclassified_error() -> ErrorClass {
    serde_json::from_str(
        r#"{"family":"unclassified","message":"boom","model":null,"provider":null}"#,
    )
    .unwrap()
}

fn server_tool_use(response: &SyntheticSearchResponse) -> &ServerToolUseBlock {
    let Some(SyntheticContentBlock::ServerToolUse(block)) = response.content.first() else {
        panic!("the first block must be server_tool_use");
    };
    block
}

fn tool_result(response: &SyntheticSearchResponse) -> &ResultBlock {
    let Some(SyntheticContentBlock::ToolResult(block)) = response.content.get(1) else {
        panic!("the second block must be web_search_tool_result");
    };
    block
}

fn text_block(response: &SyntheticSearchResponse) -> &TextBlock {
    let Some(SyntheticContentBlock::Text(block)) = response.content.last() else {
        panic!("the last block must be text");
    };
    block
}

fn result_items(block: &ResultBlock) -> &[ResultItem] {
    let ResultBlock::WebSearchToolResult { content, .. } = block;
    let ResultContent::Items(items) = content else {
        panic!("the result block must carry items");
    };
    items
}

fn error_code(block: &ResultBlock) -> WebSearchToolResultErrorCode {
    let ResultBlock::WebSearchToolResult { content, .. } = block;
    let ResultContent::Error(ResultError::WebSearchToolResultError { error_code }) = content else {
        panic!("the result block must carry an error");
    };
    *error_code
}

async fn short_circuit(
    search: &WebSearchExecution<FakeRunner, FakeGrants, FakeMetadata, FakeCapabilities>,
    tools: &[Tool],
    messages: &[Message],
    provider: Option<&str>,
) -> Option<SyntheticSearchResponse> {
    let request = ShortCircuitRequest {
        model: "claude-sonnet-4-5",
        messages,
        tools: Some(tools),
        custom_llm_provider: provider,
    };
    search.try_short_circuit_search(&request, None).await
}

#[tokio::test]
async fn native_short_circuit_emits_server_tool_use_result_and_text() {
    let messages = [user_message("What is Rust?")];
    let tools = [native_tool()];
    let search = execution(runner(search_response()), empty_capabilities());
    let response = short_circuit(&search, &tools, &messages, Some("github_copilot"))
        .await
        .unwrap();
    assert_eq!(response.message_kind, MessageKind::Message);
    assert_eq!(response.role, AssistantRole::Assistant);
    assert_eq!(response.model, "claude-sonnet-4-5");
    assert_eq!(response.stop_reason, StopReason::EndTurn);
    assert_eq!(response.stop_sequence, None);
    assert_eq!(response.usage.input_tokens, 0);
    assert_eq!(response.usage.output_tokens, 0);
    assert!(response.id.starts_with("msg_"));
    assert_eq!(response.content.len(), 3);
    let server = server_tool_use(&response);
    assert_eq!(server.name, "web_search");
    assert_eq!(server.input.query, "What is Rust?");
    assert!(server.id.starts_with("srvtoolu_"));
    let ResultBlock::WebSearchToolResult { tool_use_id, .. } = tool_result(&response);
    assert_eq!(tool_use_id, &server.id);
    let items = result_items(tool_result(&response));
    let ResultItem::WebSearchResult {
        url,
        title,
        snippet,
        page_age,
        encrypted_content,
    } = &items[0];
    assert_eq!(title, "T");
    assert_eq!(url, "https://example.com");
    assert_eq!(snippet, "S");
    assert_eq!(page_age, &None);
    assert_eq!(encrypted_content, "");
    assert_eq!(text_block(&response).block_kind, TextKind::Text);
    assert_eq!(
        text_block(&response).text,
        "Title: T\nURL: https://example.com\nSnippet: S"
    );
}

#[tokio::test]
async fn text_only_short_circuit_emits_a_single_text_block() {
    let messages = [user_message("What is Rust?")];
    let tools = [litellm_tool()];
    let search = execution(runner(search_response()), empty_capabilities());
    let response = short_circuit(&search, &tools, &messages, Some("openai"))
        .await
        .unwrap();
    assert_eq!(response.content.len(), 1);
    assert_eq!(
        text_block(&response).text,
        "Title: T\nURL: https://example.com\nSnippet: S"
    );
}

#[tokio::test]
async fn short_circuit_skips_a_provider_that_is_not_enabled() {
    let messages = [user_message("What is Rust?")];
    let tools = [native_tool()];
    let mut search = execution(runner(search_response()), empty_capabilities());
    search.enabled_providers = Some(vec!["bedrock".to_owned()]);
    assert!(
        short_circuit(&search, &tools, &messages, Some("anthropic"))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn short_circuit_skips_a_provider_that_handles_web_search_natively() {
    let messages = [user_message("What is Rust?")];
    let tools = [native_tool()];
    let capabilities = FakeCapabilities {
        native_providers: vec!["anthropic".to_owned()],
        rich_providers: Vec::new(),
    };
    let search = execution(runner(search_response()), capabilities);
    assert!(
        short_circuit(&search, &tools, &messages, Some("anthropic"))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn short_circuit_skips_a_tool_list_with_a_non_search_tool() {
    let messages = [user_message("What is Rust?")];
    let tools = [
        native_tool(),
        tool(r#"{"type":"function","function":{"name":"other"}}"#),
    ];
    let search = execution(runner(search_response()), empty_capabilities());
    assert!(
        short_circuit(&search, &tools, &messages, Some("openai"))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn short_circuit_skips_when_no_user_message_is_trailing() {
    let messages = [Message {
        role: "assistant".to_owned(),
        content: Some(Content::Text("earlier".to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }];
    let tools = [native_tool()];
    let search = execution(runner(search_response()), empty_capabilities());
    assert!(
        short_circuit(&search, &tools, &messages, Some("openai"))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn short_circuit_skips_an_empty_user_message() {
    let messages = [user_message("   ")];
    let tools = [native_tool()];
    let search = execution(runner(search_response()), empty_capabilities());
    assert!(
        short_circuit(&search, &tools, &messages, Some("openai"))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn short_circuit_skips_missing_and_empty_tool_lists() {
    let messages = [user_message("What is Rust?")];
    let search = execution(runner(search_response()), empty_capabilities());
    let none = ShortCircuitRequest {
        model: "gpt-5",
        messages: &messages,
        tools: None,
        custom_llm_provider: Some("openai"),
    };
    let empty = ShortCircuitRequest {
        model: "gpt-5",
        messages: &messages,
        tools: Some(&[]),
        custom_llm_provider: Some("openai"),
    };
    assert!(search.try_short_circuit_search(&none, None).await.is_none());
    assert!(
        search
            .try_short_circuit_search(&empty, None)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn short_circuit_accepts_any_provider_when_the_list_is_absent() {
    let messages = [user_message("What is Rust?")];
    let tools = [litellm_tool()];
    let search = execution(runner(search_response()), empty_capabilities());
    assert!(
        short_circuit(&search, &tools, &messages, None)
            .await
            .is_some()
    );
}

async fn failed_short_circuit(failure: SearchFailure, native: bool) -> SyntheticSearchResponse {
    let messages = [user_message("What is Rust?")];
    let tools = if native {
        vec![native_tool()]
    } else {
        vec![litellm_tool()]
    };
    let search = execution(failing_runner(failure), empty_capabilities());
    short_circuit(&search, &tools, &messages, Some("openai"))
        .await
        .unwrap()
}

#[tokio::test]
async fn short_circuit_reports_a_rate_limit_as_too_many_requests() {
    let failure = SearchFailure {
        class: rate_limit(),
        message: "slow down".to_owned(),
    };
    let response = failed_short_circuit(failure, true).await;
    assert_eq!(
        error_code(tool_result(&response)),
        WebSearchToolResultErrorCode::TooManyRequests
    );
    assert_eq!(text_block(&response).text, "Search failed: slow down");
}

#[tokio::test]
async fn short_circuit_reports_a_bad_request_as_invalid_tool_input() {
    let failure = SearchFailure {
        class: bad_request(),
        message: "bad input".to_owned(),
    };
    let response = failed_short_circuit(failure, true).await;
    assert_eq!(
        error_code(tool_result(&response)),
        WebSearchToolResultErrorCode::InvalidToolInput
    );
}

#[tokio::test]
async fn short_circuit_reports_any_other_failure_as_unavailable() {
    let failure = SearchFailure {
        class: unclassified_error(),
        message: "boom".to_owned(),
    };
    let response = failed_short_circuit(failure, true).await;
    assert_eq!(
        error_code(tool_result(&response)),
        WebSearchToolResultErrorCode::Unavailable
    );
    assert_eq!(text_block(&response).text, "Search failed: boom");
}

#[tokio::test]
async fn failed_short_circuit_without_a_native_tool_keeps_only_the_text_block() {
    let failure = SearchFailure {
        class: unclassified_error(),
        message: "boom".to_owned(),
    };
    let response = failed_short_circuit(failure, false).await;
    assert_eq!(response.content.len(), 1);
    assert_eq!(text_block(&response).text, "Search failed: boom");
}

#[tokio::test]
async fn server_tool_use_name_defaults_to_web_search() {
    let messages = [user_message("What is Rust?")];
    let tools = [tool(r#"{"type":"web_search_20250305"}"#)];
    let search = execution(runner(search_response()), empty_capabilities());
    let response = short_circuit(&search, &tools, &messages, Some("openai"))
        .await
        .unwrap();
    assert_eq!(server_tool_use(&response).name, "web_search");
}

#[tokio::test]
async fn server_tool_use_name_keeps_the_client_tool_name() {
    let messages = [user_message("What is Rust?")];
    let tools = [tool(
        r#"{"type":"web_search_20250305","name":"custom_search"}"#,
    )];
    let search = execution(runner(search_response()), empty_capabilities());
    let response = short_circuit(&search, &tools, &messages, Some("openai"))
        .await
        .unwrap();
    assert_eq!(server_tool_use(&response).name, "custom_search");
}

#[tokio::test]
async fn execute_search_reads_provider_and_params_from_the_router_tool() {
    let mut search = execution(runner(search_response()), empty_capabilities());
    search.search_tools = Some(vec![search_tool(
        "main",
        "tavily",
        r#","max_results":3,"country":null"#,
    )]);
    let (text, response) = search.execute_search("rust", None, None).await.unwrap();
    assert_eq!(text, "Title: T\nURL: https://example.com\nSnippet: S");
    assert_eq!(response, Some(search_response()));
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].search_provider, "tavily");
    assert_eq!(calls[0].query, SearchQuery::Single("rust".to_owned()));
    assert_eq!(calls[0].params.get("max_results").unwrap().get(), "3");
    assert!(!calls[0].params.contains_key("country"));
    assert!(!calls[0].params.contains_key("search_provider"));
    assert!(calls[0].metadata.is_none());
}

#[tokio::test]
async fn execute_search_without_a_search_tool_uses_perplexity() {
    let search = execution(runner(search_response()), empty_capabilities());
    let (text, response) = search.execute_search("rust", None, None).await.unwrap();
    assert_eq!(text, "Title: T\nURL: https://example.com\nSnippet: S");
    assert!(response.is_some());
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert_eq!(calls[0].search_provider, DEFAULT_SEARCH_PROVIDER);
    assert!(calls[0].params.is_empty());
}

#[tokio::test]
async fn execute_search_selects_the_configured_search_tool() {
    let mut search = execution(runner(search_response()), empty_capabilities());
    search.search_tool_name = Some("second".to_owned());
    search.search_tools = Some(vec![
        search_tool("first", "tavily", ""),
        search_tool("second", "exa_ai", ""),
    ]);
    search.execute_search("rust", None, None).await.unwrap();
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert_eq!(calls[0].search_provider, "exa_ai");
}

#[tokio::test]
async fn execute_search_forwards_rich_queries_to_a_supporting_provider() {
    let capabilities = FakeCapabilities {
        native_providers: Vec::new(),
        rich_providers: vec!["parallel_ai".to_owned()],
    };
    let mut search = execution(runner(search_response()), capabilities);
    search.search_tools = Some(vec![search_tool("main", "parallel_ai", "")]);
    let rich = rich_search_input(&input_json(
        r#"{"objective":"find rust docs","search_queries":["rust","cargo"]}"#,
    ));
    search
        .execute_search("rust", None, rich.as_ref())
        .await
        .unwrap();
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert_eq!(
        calls[0].query,
        SearchQuery::Multiple(vec!["rust".to_owned(), "cargo".to_owned()])
    );
    assert_eq!(
        calls[0].params.get("objective").unwrap().get(),
        "\"find rust docs\""
    );
}

#[tokio::test]
async fn execute_search_keeps_the_query_string_for_other_providers() {
    let mut search = execution(runner(search_response()), empty_capabilities());
    search.search_tools = Some(vec![search_tool("main", "perplexity", "")]);
    let rich = rich_search_input(&input_json(
        r#"{"objective":"find rust docs","search_queries":["rust","cargo"]}"#,
    ));
    search
        .execute_search("rust", None, rich.as_ref())
        .await
        .unwrap();
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert_eq!(calls[0].query, SearchQuery::Single("rust".to_owned()));
    assert!(!calls[0].params.contains_key("objective"));
}

#[tokio::test]
async fn execute_search_keeps_a_configured_objective() {
    let capabilities = FakeCapabilities {
        native_providers: Vec::new(),
        rich_providers: vec!["parallel_ai".to_owned()],
    };
    let mut search = execution(runner(search_response()), capabilities);
    search.search_tools = Some(vec![search_tool(
        "main",
        "parallel_ai",
        r#","objective":"configured""#,
    )]);
    let rich = rich_search_input(&input_json(r#"{"objective":"from the model"}"#));
    search
        .execute_search("rust", None, rich.as_ref())
        .await
        .unwrap();
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert_eq!(
        calls[0].params.get("objective").unwrap().get(),
        "\"configured\""
    );
}

#[tokio::test]
async fn execute_search_forwards_session_and_trace_kwargs() {
    let search = execution(runner(search_response()), empty_capabilities());
    let kwargs: SearchHookKwargs =
        serde_json::from_str(r#"{"litellm_session_id":"session-1","litellm_trace_id":"trace-1"}"#)
            .unwrap();
    search
        .execute_search("rust", Some(&kwargs), None)
        .await
        .unwrap();
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert_eq!(
        calls[0].params.get("litellm_session_id").unwrap().get(),
        "\"session-1\""
    );
    assert_eq!(
        calls[0].params.get("litellm_trace_id").unwrap().get(),
        "\"trace-1\""
    );
    assert!(calls[0].metadata.is_none());
}

#[tokio::test]
async fn execute_search_builds_metadata_only_when_a_caller_is_present() {
    let mut search = execution(runner(search_response()), empty_capabilities());
    search.search_tools = Some(vec![search_tool("main", "tavily", "")]);
    let without_caller: SearchHookKwargs =
        serde_json::from_str(r#"{"litellm_call_id":"call-1"}"#).unwrap();
    search
        .execute_search("rust", Some(&without_caller), None)
        .await
        .unwrap();
    let with_caller: SearchHookKwargs =
        serde_json::from_str(r#"{"metadata":{"user_api_key_auth":{"team_id":null}}}"#).unwrap();
    search
        .execute_search("rust", Some(&with_caller), None)
        .await
        .unwrap();
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert!(calls[0].metadata.is_none());
    let metadata = calls[1].metadata.as_ref().unwrap();
    let model_group = metadata.get("model_group").unwrap();
    let SearchMetadataValue::Text(group_name) = model_group else {
        panic!("model_group must be text");
    };
    assert_eq!(group_name, "main");
    let api_key = metadata.get("user_api_key").unwrap();
    let SearchMetadataValue::Text(logged_key) = api_key else {
        panic!("user_api_key must be text");
    };
    assert_eq!(logged_key, "sk-test");
}

#[tokio::test]
async fn execute_search_fails_before_the_search_when_the_grant_is_denied() {
    let search = denied_execution(vec![search_tool("main", "tavily", "")]);
    let failure = search.execute_search("rust", None, None).await.unwrap_err();
    assert_eq!(failure.message, "search tool access denied");
    assert_eq!(
        search_error_code(&failure.class),
        WebSearchToolResultErrorCode::Unavailable
    );
    assert!(search.runner.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn short_circuit_search_outcome_converts_success_and_failure() {
    let search = execution(runner(search_response()), empty_capabilities());
    let succeeded = search.short_circuit_search_outcome("rust", None).await;
    let SearchOutcome::Succeeded(success) = succeeded else {
        panic!("expected a successful outcome");
    };
    assert_eq!(
        success.text,
        "Title: T\nURL: https://example.com\nSnippet: S"
    );
    assert!(success.response.is_some());

    let failing = execution(
        failing_runner(unclassified("down".to_owned())),
        empty_capabilities(),
    );
    let failed = failing.short_circuit_search_outcome("rust", None).await;
    let SearchOutcome::Failed(outcome) = failed else {
        panic!("expected a failed outcome");
    };
    assert_eq!(
        outcome.error_code,
        WebSearchToolResultErrorCode::Unavailable
    );
    assert_eq!(outcome.message, "down");
}

#[tokio::test]
async fn provider_supports_rich_search_follows_the_capability_port() {
    let capabilities = FakeCapabilities {
        native_providers: Vec::new(),
        rich_providers: vec!["parallel_ai".to_owned()],
    };
    let search = execution(runner(search_response()), capabilities);
    assert!(search.provider_supports_rich_search(Some("parallel_ai")));
    assert!(!search.provider_supports_rich_search(Some("perplexity")));
    assert!(!search.provider_supports_rich_search(Some("")));
    assert!(!search.provider_supports_rich_search(None));
}

#[tokio::test]
async fn provider_supports_rich_search_never_asks_about_an_empty_provider() {
    let capabilities = FakeCapabilities {
        native_providers: Vec::new(),
        rich_providers: vec![String::new(), "parallel_ai".to_owned()],
    };
    let search = execution(runner(search_response()), capabilities);
    assert!(!search.provider_supports_rich_search(Some("")));
    assert!(search.provider_supports_rich_search(Some("parallel_ai")));
}

#[tokio::test]
async fn execute_search_keeps_the_query_string_for_an_empty_query_list() {
    let capabilities = FakeCapabilities {
        native_providers: Vec::new(),
        rich_providers: vec!["parallel_ai".to_owned()],
    };
    let mut search = execution(runner(search_response()), capabilities);
    search.search_tools = Some(vec![search_tool("main", "parallel_ai", "")]);
    let rich = RichWebSearchInput {
        objective: None,
        search_queries: Some(Vec::new()),
    };
    search
        .execute_search("rust", None, Some(&rich))
        .await
        .unwrap();
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert_eq!(calls[0].query, SearchQuery::Single("rust".to_owned()));
}

#[tokio::test]
async fn execute_search_skips_an_empty_objective() {
    let capabilities = FakeCapabilities {
        native_providers: Vec::new(),
        rich_providers: vec!["parallel_ai".to_owned()],
    };
    let mut search = execution(runner(search_response()), capabilities);
    search.search_tools = Some(vec![search_tool("main", "parallel_ai", "")]);
    let rich = RichWebSearchInput {
        objective: Some(String::new()),
        search_queries: None,
    };
    search
        .execute_search("rust", None, Some(&rich))
        .await
        .unwrap();
    let calls = take(&mut *search.runner.calls.lock().unwrap());
    assert!(!calls[0].params.contains_key("objective"));
}

fn input_json(source: &str) -> Input {
    Input::from(serde_json::from_str::<Box<RawValue>>(source).unwrap())
}

#[test]
fn rich_search_input_returns_none_for_an_object_without_rich_fields() {
    assert_eq!(rich_search_input(&input_json(r#"{"query":"rust"}"#)), None);
}

#[test]
fn rich_search_input_returns_none_for_a_non_object_input() {
    assert_eq!(rich_search_input(&input_json(r#""rust""#)), None);
}

#[test]
fn rich_search_input_reads_the_objective_and_queries() {
    let rich = rich_search_input(&input_json(
        r#"{"objective":"find rust","search_queries":["rust","cargo"]}"#,
    ))
    .unwrap();
    assert_eq!(rich.objective.as_deref(), Some("find rust"));
    assert_eq!(
        rich.search_queries,
        Some(vec!["rust".to_owned(), "cargo".to_owned()])
    );
}

#[test]
fn rich_search_input_keeps_either_field_alone() {
    let objective_only = rich_search_input(&input_json(r#"{"objective":"find rust"}"#)).unwrap();
    assert_eq!(objective_only.objective.as_deref(), Some("find rust"));
    assert_eq!(objective_only.search_queries, None);
    let queries_only = rich_search_input(&input_json(r#"{"search_queries":["rust"]}"#)).unwrap();
    assert_eq!(queries_only.objective, None);
    assert_eq!(queries_only.search_queries, Some(vec!["rust".to_owned()]));
}

#[test]
fn rich_search_input_limits_the_queries_to_five() {
    let rich = rich_search_input(&input_json(
        r#"{"search_queries":["one","two","three","four","five","six"]}"#,
    ))
    .unwrap();
    assert_eq!(
        rich.search_queries,
        Some(vec![
            "one".to_owned(),
            "two".to_owned(),
            "three".to_owned(),
            "four".to_owned(),
            "five".to_owned()
        ])
    );
}

#[test]
fn rich_search_input_drops_blank_and_non_string_values() {
    let rich = rich_search_input(&input_json(
        r#"{"objective":"  ","search_queries":["rust","  ",7,"cargo"]}"#,
    ))
    .unwrap();
    assert_eq!(rich.objective, None);
    assert_eq!(
        rich.search_queries,
        Some(vec!["rust".to_owned(), "cargo".to_owned()])
    );
}

#[test]
fn rich_search_input_returns_none_when_nothing_valid_remains() {
    assert_eq!(
        rich_search_input(&input_json(
            r#"{"objective":" ","search_queries":[""," " ]}"#
        )),
        None
    );
    assert_eq!(
        rich_search_input(&input_json(r#"{"search_queries":"rust"}"#)),
        None
    );
    assert_eq!(
        rich_search_input(&input_json(r#"{"search_queries":[]}"#)),
        None
    );
}

#[test]
fn empty_search_result_carries_the_fixed_text() {
    let (text, response) = create_empty_search_result();
    assert_eq!(text, "No search query provided");
    assert!(response.is_none());
}
