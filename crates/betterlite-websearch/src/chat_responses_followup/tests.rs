#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::absolute_paths,
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        clippy::disallowed_types,
        clippy::needless_pass_by_value,
        clippy::shadow_reuse,
        clippy::too_many_lines,
        reason = "the fixtures state the provider JSON LiteLLM hands the builders, and each test reads as one scenario"
    )
)]
mod forwarding;
mod port;

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap, get_text};
use rstest::rstest;
use serde_json::{Value, json, value::to_raw_value};

use super::ChatResponsesFollowup;
use super::chat::{FollowupRequest, build_chat_patch};
use super::responses::{ResponsesInput, ResponsesRequest, build_responses_patch};
use super::search::{SearchExecutor, SearchSurface, create_empty_search_result, search_batch};
use crate::call_transformation::WebSearchTransformation;
use crate::call_transformation::outcome::{SearchResult, search_outcome, search_outcome_text};
use crate::call_transformation::wire::{Call, CallFunction, CallKind, Input, Response};
use crate::execution::search::unclassified;
use crate::records::RichWebSearchInput;

struct RecordedSearch {
    query: String,
    rich: Option<RichWebSearchInput>,
    marker: Option<String>,
}

struct ScriptedSearch {
    scripts: Mutex<Vec<SearchResult>>,
    searches: Mutex<Vec<RecordedSearch>>,
}

impl ScriptedSearch {
    fn with_scripts(scripts: Vec<SearchResult>) -> Self {
        Self {
            scripts: Mutex::new(scripts),
            searches: Mutex::new(Vec::new()),
        }
    }

    fn successful(texts: &[&str]) -> Self {
        let scripts = texts
            .iter()
            .map(|text| SearchResult::Succeeded {
                text: (*text).to_owned(),
                response: None,
            })
            .collect();
        Self::with_scripts(scripts)
    }

    fn recorded(&self) -> Vec<RecordedSearch> {
        let mut searches = self.searches.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *searches)
    }
}

#[async_trait]
impl SearchExecutor for ScriptedSearch {
    async fn execute_search(
        &self,
        query: &str,
        kwargs: &JsonMap,
        rich: Option<RichWebSearchInput>,
    ) -> SearchResult {
        if let Ok(mut searches) = self.searches.lock() {
            searches.push(RecordedSearch {
                query: query.to_owned(),
                rich,
                marker: get_text(kwargs, "marker"),
            });
        }
        let mut scripts = self.scripts.lock().unwrap_or_else(PoisonError::into_inner);
        if scripts.is_empty() {
            return SearchResult::Failed(unclassified("the test scripts no search".to_owned()));
        }
        scripts.remove(0)
    }
}

fn map(value: Value) -> JsonMap {
    serde_json::from_str(&value.to_string()).unwrap()
}

fn message(value: Value) -> FollowupMessage {
    FollowupMessage(to_raw_value(&value).unwrap())
}

fn calls_from(response: Value, format: &str) -> Vec<Call> {
    let response: Response = serde_json::from_str(&response.to_string()).unwrap();
    let (_, calls) = WebSearchTransformation::transform_request(&response, false, format).unwrap();
    calls
}

fn chat_calls(tool_calls: Value) -> Vec<Call> {
    calls_from(
        json!({"choices": [{"message": {"tool_calls": tool_calls}}]}),
        "openai",
    )
}

fn responses_calls(output: Value) -> Vec<Call> {
    calls_from(json!({"output": output}), "responses")
}

fn openai_call(id: &str, arguments: Value) -> Value {
    json!({
        "id": id,
        "type": "function",
        "function": {"name": "litellm_web_search", "arguments": arguments},
    })
}

fn function_call(call_id: Option<&str>, arguments: &str) -> Value {
    json!({
        "type": "function_call",
        "id": "fc_1",
        "name": "litellm_web_search",
        "call_id": call_id,
        "arguments": arguments,
    })
}

fn manual_call(input: Input, function_arguments: Input) -> Call {
    Call {
        id: Some("call_1".to_owned()),
        kind: CallKind::Function,
        name: "litellm_web_search".to_owned(),
        input,
        function: Some(CallFunction {
            name: "litellm_web_search".to_owned(),
            arguments: function_arguments,
        }),
        arguments: None,
        call_id: None,
    }
}

fn messages_value(messages: &[FollowupMessage]) -> Value {
    serde_json::to_value(messages).unwrap()
}

fn follows(searcher: &Arc<ScriptedSearch>) -> ChatResponsesFollowup {
    let shared = Arc::clone(searcher);
    let search: Arc<dyn SearchExecutor> = shared;
    ChatResponsesFollowup { search }
}

#[tokio::test]
async fn search_batch_keeps_call_order_and_isolates_a_failure() {
    let searcher = Arc::new(ScriptedSearch::with_scripts(vec![
        SearchResult::Failed(unclassified("boom".to_owned())),
        SearchResult::Succeeded {
            text: "second".to_owned(),
            response: None,
        },
    ]));
    let calls = chat_calls(json!([
        openai_call("call_1", json!({"query": "first"})),
        openai_call("call_2", json!({"query": "second"})),
    ]));
    let kwargs = map(json!({}));
    let outcomes = search_batch(searcher.as_ref(), &calls, &kwargs, SearchSurface::Chat).await;
    let texts: Vec<String> = outcomes.iter().map(search_outcome_text).collect();
    assert_eq!(
        texts,
        vec!["Search failed: boom".to_owned(), "second".to_owned()]
    );
    let recorded = searcher.recorded();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].query, "first");
    assert_eq!(recorded[1].query, "second");
}

#[tokio::test]
async fn search_batch_skips_the_executor_for_a_call_without_a_query() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([
        openai_call("call_1", json!({})),
        openai_call("call_2", json!({"query": "second"})),
    ]));
    let kwargs = map(json!({}));
    let outcomes = search_batch(searcher.as_ref(), &calls, &kwargs, SearchSurface::Chat).await;
    assert_eq!(
        search_outcome_text(&outcomes[0]),
        "No search query provided"
    );
    assert_eq!(search_outcome_text(&outcomes[1]), "result");
    let recorded = searcher.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].query, "second");
}

#[tokio::test]
async fn search_batch_skips_an_empty_query() {
    let searcher = Arc::new(ScriptedSearch::successful(&[]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": ""}))]));
    let kwargs = map(json!({}));
    let outcomes = search_batch(searcher.as_ref(), &calls, &kwargs, SearchSurface::Chat).await;
    assert_eq!(
        search_outcome_text(&outcomes[0]),
        "No search query provided"
    );
    assert!(searcher.recorded().is_empty());
}

#[rstest]
#[case::object_arguments(json!({"query": "fallback"}), Some("fallback"))]
#[case::non_object_arguments(json!("not an object"), None)]
#[tokio::test]
async fn search_batch_uses_function_arguments_when_input_is_not_an_object(
    #[case] arguments: Value,
    #[case] expected: Option<&str>,
) {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let call = manual_call(
        Input::Other(to_raw_value(&json!("not an object")).unwrap()),
        Input::from(to_raw_value(&arguments).unwrap()),
    );
    let kwargs = map(json!({}));
    let outcomes = search_batch(searcher.as_ref(), &[call], &kwargs, SearchSurface::Chat).await;
    let queries: Vec<String> = searcher
        .recorded()
        .into_iter()
        .map(|search| search.query)
        .collect();
    assert_eq!(
        queries,
        expected.map(str::to_owned).into_iter().collect::<Vec<_>>()
    );
    let expected_text = expected.map_or("No search query provided", |_| "result");
    assert_eq!(search_outcome_text(&outcomes[0]), expected_text);
}

#[tokio::test]
async fn search_batch_responses_surface_ignores_function_arguments() {
    let call = manual_call(
        Input::Other(to_raw_value(&json!("not an object")).unwrap()),
        Input::from(to_raw_value(&json!({"query": "fallback"})).unwrap()),
    );
    let kwargs = map(json!({}));
    let chat = Arc::new(ScriptedSearch::successful(&["result"]));
    let chat_outcomes = search_batch(
        chat.as_ref(),
        std::slice::from_ref(&call),
        &kwargs,
        SearchSurface::Chat,
    )
    .await;
    assert_eq!(search_outcome_text(&chat_outcomes[0]), "result");
    let responses = Arc::new(ScriptedSearch::successful(&[]));
    let responses_outcomes = search_batch(
        responses.as_ref(),
        &[call],
        &kwargs,
        SearchSurface::Responses,
    )
    .await;
    assert_eq!(
        search_outcome_text(&responses_outcomes[0]),
        "No search query provided"
    );
    assert!(responses.recorded().is_empty());
}

#[tokio::test]
async fn search_batch_forwards_rich_input_and_kwargs() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call(
        "call_1",
        json!({"query": "weather", "objective": "find", "search_queries": ["a", "b"]}),
    )]));
    let kwargs = map(json!({"marker": "kept"}));
    let outcomes = search_batch(searcher.as_ref(), &calls, &kwargs, SearchSurface::Chat).await;
    assert_eq!(search_outcome_text(&outcomes[0]), "result");
    let recorded = searcher.recorded();
    assert_eq!(
        recorded[0].rich,
        Some(RichWebSearchInput {
            objective: Some("find".to_owned()),
            search_queries: Some(vec!["a".to_owned(), "b".to_owned()]),
        })
    );
    assert_eq!(recorded[0].marker.as_deref(), Some("kept"));
}

#[tokio::test]
async fn search_batch_with_no_calls_returns_no_outcomes() {
    let searcher = Arc::new(ScriptedSearch::successful(&[]));
    let kwargs = map(json!({}));
    let outcomes = search_batch(searcher.as_ref(), &[], &kwargs, SearchSurface::Chat).await;
    assert!(outcomes.is_empty());
    assert!(searcher.recorded().is_empty());
}

#[test]
fn search_batch_empty_result_has_the_litellm_text() {
    assert_eq!(
        search_outcome_text(&search_outcome(create_empty_search_result())),
        "No search query provided"
    );
}

#[tokio::test]
async fn chat_patch_finds_the_query_in_the_input_object() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": "weather"}))]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_chat_patch(searcher.as_ref(), &calls, "openai", &request)
        .await
        .unwrap();
    assert_eq!(searcher.recorded()[0].query, "weather");
    let value = messages_value(&patch.messages.unwrap());
    assert_eq!(value[0], json!({"role": "user", "content": "hi"}));
    assert_eq!(value[1]["role"], "assistant");
    assert_eq!(
        value[2],
        json!({"role": "tool", "tool_call_id": "call_1", "content": "result"})
    );
}

#[tokio::test]
async fn chat_patch_forwards_the_rich_input() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call(
        "call_1",
        json!({"query": "weather", "objective": "find", "search_queries": ["a"]}),
    )]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    build_chat_patch(searcher.as_ref(), &calls, "openai", &request)
        .await
        .unwrap();
    let recorded = searcher.recorded();
    assert_eq!(
        recorded[0].rich,
        Some(RichWebSearchInput {
            objective: Some("find".to_owned()),
            search_queries: Some(vec!["a".to_owned()]),
        })
    );
}

#[tokio::test]
async fn chat_patch_without_a_query_uses_the_empty_result() {
    let searcher = Arc::new(ScriptedSearch::successful(&[]));
    let calls = chat_calls(json!([openai_call("call_1", json!({}))]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_chat_patch(searcher.as_ref(), &calls, "openai", &request)
        .await
        .unwrap();
    assert!(searcher.recorded().is_empty());
    let value = messages_value(&patch.messages.unwrap());
    assert_eq!(value[2]["content"], "No search query provided");
}

#[tokio::test]
async fn chat_patch_appends_the_assistant_and_tool_messages_for_openai() {
    let searcher = Arc::new(ScriptedSearch::successful(&["one", "two"]));
    let calls = chat_calls(json!([
        openai_call("call_1", json!({"query": "first"})),
        openai_call("call_2", json!({"query": "second"})),
    ]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_chat_patch(searcher.as_ref(), &calls, "openai", &request)
        .await
        .unwrap();
    let value = messages_value(&patch.messages.unwrap());
    assert_eq!(value.as_array().unwrap().len(), 4);
    assert_eq!(value[1]["role"], "assistant");
    assert_eq!(value[1]["tool_calls"][0]["id"], "call_1");
    assert_eq!(value[1]["tool_calls"][1]["id"], "call_2");
    assert_eq!(value[2]["content"], "one");
    assert_eq!(value[3]["content"], "two");
}

#[rstest]
#[case::anthropic("anthropic")]
#[case::responses("responses")]
#[tokio::test]
async fn chat_patch_appends_assistant_and_user_messages_for_other_formats(
    #[case] response_format: &str,
) {
    let searcher = Arc::new(ScriptedSearch::successful(&["one", "two"]));
    let calls = chat_calls(json!([
        openai_call("call_1", json!({"query": "first"})),
        openai_call("call_2", json!({"query": "second"})),
    ]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_chat_patch(searcher.as_ref(), &calls, response_format, &request)
        .await
        .unwrap();
    let value = messages_value(&patch.messages.unwrap());
    assert_eq!(value.as_array().unwrap().len(), 3);
    assert_eq!(value[1]["role"], "assistant");
    assert_eq!(value[1]["content"][0]["type"], "tool_use");
    assert_eq!(value[2]["role"], "user");
    assert_eq!(value[2]["content"][0]["type"], "tool_result");
    assert_eq!(value[2]["content"][0]["content"], "one");
    assert_eq!(value[2]["content"][1]["content"], "two");
}

#[rstest]
#[case::provider_prefix(json!({"custom_llm_provider": "azure"}), "gpt-4o", "azure/gpt-4o")]
#[case::model_starts_with_provider(
    json!({"custom_llm_provider": "azure"}),
    "azure-gpt",
    "azure-gpt"
)]
#[case::model_has_a_slash(json!({"custom_llm_provider": "azure"}), "openai/gpt-4o", "openai/gpt-4o")]
#[case::no_provider(json!({}), "gpt-4o", "gpt-4o")]
#[case::provider_not_a_string(json!({"custom_llm_provider": 3}), "gpt-4o", "gpt-4o")]
#[tokio::test]
async fn chat_patch_prefixes_the_model(
    #[case] kwargs: Value,
    #[case] model: &str,
    #[case] expected: &str,
) {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": "weather"}))]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({}));
    let kwargs = map(kwargs);
    let request = FollowupRequest {
        model,
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_chat_patch(searcher.as_ref(), &calls, "openai", &request)
        .await
        .unwrap();
    assert_eq!(patch.model.as_deref(), Some(expected));
}

#[rstest]
#[case::tools_array(
    json!({"tools": [{"type": "function"}], "temperature": 0.2}),
    json!({"temperature": 0.2, "tools": [{"type": "function"}]})
)]
#[case::tools_null(json!({"tools": null, "temperature": 0.2}), json!({"temperature": 0.2}))]
#[case::tools_absent(json!({"temperature": 0.2}), json!({"temperature": 0.2}))]
#[tokio::test]
async fn chat_patch_keeps_tools_in_optional_params(
    #[case] optional_params: Value,
    #[case] expected: Value,
) {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": "weather"}))]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(optional_params);
    let kwargs = map(json!({}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_chat_patch(searcher.as_ref(), &calls, "openai", &request)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&patch.optional_params).unwrap(),
        expected
    );
    assert!(patch.tools.is_none());
}

#[tokio::test]
async fn chat_patch_filters_optional_params_and_kwargs() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": "weather"}))]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({
        "tools": [{"type": "function"}],
        "tool_choice": "auto",
        "extra_body": {"trace": true},
        "model_alias_map": {},
        "stream_response": {},
        "custom_prompt_dict": {},
        "temperature": 0.2,
        "stream": true,
    }));
    let kwargs = map(json!({
        "_websearch_interception": true,
        "_websearch_interception_emit_native_blocks": true,
        "_agentic_loop_api_surface": "chat_completions",
        "acompletion": true,
        "litellm_logging_obj": {},
        "custom_llm_provider": "azure",
        "model_alias_map": {},
        "stream_response": {},
        "custom_prompt_dict": {},
        "_compression_interception_x": true,
        "metadata": {"session_id": "s"},
    }));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_chat_patch(searcher.as_ref(), &calls, "openai", &request)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&patch.optional_params).unwrap(),
        json!({"temperature": 0.2, "stream": true, "tools": [{"type": "function"}]})
    );
    assert_eq!(
        serde_json::to_value(&patch.kwargs).unwrap(),
        json!({"_agentic_loop_api_surface": "chat_completions", "_compression_interception_x": true, "metadata": {"session_id": "s"}})
    );
    assert_eq!(patch.model.as_deref(), Some("azure/gpt-4o"));
}

#[tokio::test]
async fn responses_patch_turns_text_input_into_a_user_item() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = responses_calls(json!([function_call(
        Some("call_1"),
        "{\"query\":\"weather\"}"
    )]));
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let input = ResponsesInput::Text("find weather".to_owned());
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_responses_patch(searcher.as_ref(), &calls, &request)
        .await
        .unwrap();
    let value = messages_value(&patch.messages.unwrap());
    assert_eq!(value[0], json!({"role": "user", "content": "find weather"}));
    assert_eq!(value[1]["type"], "function_call");
}

#[tokio::test]
async fn responses_patch_copies_list_input_items() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = responses_calls(json!([function_call(
        Some("call_1"),
        "{\"query\":\"weather\"}"
    )]));
    let messages = vec![message(json!({"role": "user", "content": "keep"}))];
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let input = ResponsesInput::Items(messages);
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_responses_patch(searcher.as_ref(), &calls, &request)
        .await
        .unwrap();
    let value = messages_value(&patch.messages.unwrap());
    assert_eq!(value[0], json!({"role": "user", "content": "keep"}));
    assert_eq!(value.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn responses_patch_adds_function_call_and_output_items_in_order() {
    let searcher = Arc::new(ScriptedSearch::successful(&["one", "two"]));
    let calls = responses_calls(json!([
        function_call(Some("call_1"), "{\"query\":\"first\"}"),
        function_call(Some("call_2"), "{\"query\":\"second\"}"),
    ]));
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let input = ResponsesInput::Items(Vec::new());
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_responses_patch(searcher.as_ref(), &calls, &request)
        .await
        .unwrap();
    let value = messages_value(&patch.messages.unwrap());
    assert_eq!(value.as_array().unwrap().len(), 4);
    assert_eq!(
        value[0],
        json!({"type": "function_call", "call_id": "call_1", "name": "litellm_web_search", "arguments": "{\"query\":\"first\"}"})
    );
    assert_eq!(
        value[1],
        json!({"type": "function_call_output", "call_id": "call_1", "output": "one"})
    );
    assert_eq!(value[2]["call_id"], "call_2");
    assert_eq!(value[2]["arguments"], "{\"query\":\"second\"}");
    assert_eq!(value[3]["output"], "two");
    assert_eq!(
        searcher
            .recorded()
            .into_iter()
            .map(|search| search.query)
            .collect::<Vec<_>>(),
        vec!["first".to_owned(), "second".to_owned()]
    );
}

#[tokio::test]
async fn responses_patch_missing_call_id_and_arguments_are_null_and_empty() {
    let searcher = Arc::new(ScriptedSearch::with_scripts(vec![
        SearchResult::Succeeded {
            text: "result".to_owned(),
            response: None,
        },
    ]));
    let calls = vec![Call {
        id: None,
        kind: CallKind::FunctionCall,
        name: "litellm_web_search".to_owned(),
        input: Input::default(),
        function: None,
        arguments: None,
        call_id: None,
    }];
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let input = ResponsesInput::Items(Vec::new());
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_responses_patch(searcher.as_ref(), &calls, &request)
        .await
        .unwrap();
    let value = messages_value(&patch.messages.unwrap());
    assert_eq!(
        value[0],
        json!({"type": "function_call", "call_id": null, "name": "litellm_web_search", "arguments": ""})
    );
    assert_eq!(
        value[1],
        json!({"type": "function_call_output", "call_id": null, "output": "No search query provided"})
    );
}

#[rstest]
#[case::array(json!({"tools": [{"type": "function"}]}), Some(json!([{"type": "function"}])))]
#[case::object(json!({"tools": {"type": "function"}}), None)]
#[case::string(json!({"tools": "none"}), None)]
#[case::null(json!({"tools": null}), None)]
#[case::absent(json!({}), None)]
#[tokio::test]
async fn responses_patch_moves_array_tools_into_patch_tools(
    #[case] optional_params: Value,
    #[case] expected: Option<Value>,
) {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = responses_calls(json!([function_call(
        Some("call_1"),
        "{\"query\":\"weather\"}"
    )]));
    let optional_params = map(optional_params);
    let kwargs = map(json!({}));
    let input = ResponsesInput::Items(Vec::new());
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_responses_patch(searcher.as_ref(), &calls, &request)
        .await
        .unwrap();
    assert_eq!(
        patch
            .tools
            .map(|tools| serde_json::to_value(tools).unwrap()),
        expected
    );
    assert!(!patch.optional_params.contains_key("tools"));
}

#[tokio::test]
async fn responses_patch_filters_optional_params() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = responses_calls(json!([function_call(
        Some("call_1"),
        "{\"query\":\"weather\"}"
    )]));
    let optional_params = map(json!({
        "tools": [{"type": "function"}],
        "tool_choice": "auto",
        "stream": true,
        "model_alias_map": {},
        "stream_response": {},
        "custom_prompt_dict": {},
        "extra_body": {"trace": true},
        "temperature": 0.2,
    }));
    let kwargs = map(json!({}));
    let input = ResponsesInput::Items(Vec::new());
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_responses_patch(searcher.as_ref(), &calls, &request)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&patch.optional_params).unwrap(),
        json!({"extra_body": {"trace": true}, "temperature": 0.2})
    );
}

#[tokio::test]
async fn responses_patch_filters_kwargs() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = responses_calls(json!([function_call(
        Some("call_1"),
        "{\"query\":\"weather\"}"
    )]));
    let optional_params = map(json!({}));
    let kwargs = map(json!({
        "_websearch_interception": true,
        "_websearch_interception_emit_native_blocks": true,
        "_agentic_loop_api_surface": "responses",
        "litellm_logging_obj": {},
        "acompletion": true,
        "custom_llm_provider": "azure",
        "model_alias_map": {},
        "stream_response": {},
        "custom_prompt_dict": {},
        "metadata": {"session_id": "s"},
    }));
    let input = ResponsesInput::Items(Vec::new());
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_responses_patch(searcher.as_ref(), &calls, &request)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&patch.kwargs).unwrap(),
        json!({"stream_response": {}, "custom_prompt_dict": {}, "metadata": {"session_id": "s"}})
    );
    assert_eq!(patch.model.as_deref(), Some("azure/gpt-4o"));
}

#[rstest]
#[case::no_startswith_test(json!({"custom_llm_provider": "azure"}), "azure-gpt", "azure/azure-gpt")]
#[case::no_provider(json!({}), "gpt-4o", "gpt-4o")]
#[case::model_has_a_slash(json!({"custom_llm_provider": "azure"}), "openai/gpt-4o", "openai/gpt-4o")]
#[case::provider_not_a_string(json!({"custom_llm_provider": true}), "gpt-4o", "gpt-4o")]
#[tokio::test]
async fn responses_patch_prefixes_the_model(
    #[case] kwargs: Value,
    #[case] model: &str,
    #[case] expected: &str,
) {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = responses_calls(json!([function_call(
        Some("call_1"),
        "{\"query\":\"weather\"}"
    )]));
    let optional_params = map(json!({}));
    let kwargs = map(kwargs);
    let input = ResponsesInput::Items(Vec::new());
    let request = ResponsesRequest {
        model,
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_responses_patch(searcher.as_ref(), &calls, &request)
        .await
        .unwrap();
    assert_eq!(patch.model.as_deref(), Some(expected));
}

#[tokio::test]
async fn responses_patch_uses_the_empty_result_for_a_queryless_call() {
    let searcher = Arc::new(ScriptedSearch::successful(&[]));
    let call = manual_call(
        Input::Other(to_raw_value(&json!("not an object")).unwrap()),
        Input::from(to_raw_value(&json!({"query": "fallback"})).unwrap()),
    );
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let input = ResponsesInput::Items(Vec::new());
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let patch = build_responses_patch(searcher.as_ref(), &[call], &request)
        .await
        .unwrap();
    assert!(searcher.recorded().is_empty());
    let value = messages_value(&patch.messages.unwrap());
    assert_eq!(value[1]["output"], "No search query provided");
}

#[rstest]
#[case::openai("openai")]
#[case::anthropic("anthropic")]
#[tokio::test]
async fn followup_plan_chat_carries_websearch_metadata(#[case] response_format: &str) {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": "weather"}))]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let plan = follows(&searcher)
        .build_chat_plan(&calls, response_format, &request)
        .await
        .unwrap();
    assert!(plan.run_agentic_loop);
    assert!(plan.request_patch.is_some());
    assert!(plan.response_override.is_none());
    assert!(!plan.terminate);
    assert!(plan.stop_reason.is_none());
    assert_eq!(
        serde_json::to_value(&plan.metadata).unwrap(),
        json!({"tool_type": "websearch", "response_format": response_format})
    );
}

#[tokio::test]
async fn followup_plan_responses_carries_websearch_metadata() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = responses_calls(json!([function_call(
        Some("call_1"),
        "{\"query\":\"weather\"}"
    )]));
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let input = ResponsesInput::Items(Vec::new());
    let request = ResponsesRequest {
        model: "gpt-4o",
        input: &input,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let plan = follows(&searcher)
        .build_responses_plan(&calls, &request)
        .await
        .unwrap();
    assert!(plan.run_agentic_loop);
    assert!(plan.request_patch.is_some());
    assert_eq!(
        serde_json::to_value(&plan.metadata).unwrap(),
        json!({"tool_type": "websearch", "response_format": "responses"})
    );
}

#[tokio::test]
async fn chat_rerun_lays_patch_params_over_request_params_and_drops_tool_choice() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": "weather"}))]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({
        "temperature": 0.5,
        "top_p": 0.9,
        "tools": [{"type": "function"}],
        "tool_choice": "auto",
        "extra_body": {"trace": true},
    }));
    let kwargs = map(json!({"_agentic_loop_api_surface": "chat_completions"}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let rerun = follows(&searcher)
        .build_chat_rerun(&calls, "openai", &request)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&rerun.params).unwrap(),
        json!({"temperature": 0.5, "top_p": 0.9, "tools": [{"type": "function"}], "extra_body": {"trace": true}})
    );
    assert_eq!(
        serde_json::to_value(&rerun.kwargs).unwrap(),
        json!({"_agentic_loop_api_surface": "chat_completions"})
    );
    assert_eq!(rerun.model, "gpt-4o");
}

#[tokio::test]
async fn chat_rerun_uses_the_prefixed_patch_model() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": "weather"}))]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({}));
    let kwargs = map(json!({"custom_llm_provider": "azure", "acompletion": true}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let rerun = follows(&searcher)
        .build_chat_rerun(&calls, "openai", &request)
        .await
        .unwrap();
    assert_eq!(rerun.model, "azure/gpt-4o");
    assert!(rerun.kwargs.is_empty());
}

#[tokio::test]
async fn chat_rerun_keeps_the_followup_messages() {
    let searcher = Arc::new(ScriptedSearch::successful(&["result"]));
    let calls = chat_calls(json!([openai_call("call_1", json!({"query": "weather"}))]));
    let messages = [message(json!({"role": "user", "content": "hi"}))];
    let optional_params = map(json!({}));
    let kwargs = map(json!({}));
    let request = FollowupRequest {
        model: "gpt-4o",
        messages: &messages,
        optional_params: &optional_params,
        kwargs: &kwargs,
    };
    let rerun = follows(&searcher)
        .build_chat_rerun(&calls, "openai", &request)
        .await
        .unwrap();
    let value = messages_value(&rerun.messages);
    assert_eq!(value.as_array().unwrap().len(), 3);
    assert_eq!(value[0], json!({"role": "user", "content": "hi"}));
    assert_eq!(value[1]["role"], "assistant");
    assert_eq!(
        value[2],
        json!({"role": "tool", "tool_call_id": "call_1", "content": "result"})
    );
}
