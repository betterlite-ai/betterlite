#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_context::agentic_loop_plan::{AgenticLoopRequestPatch, FollowupMessage, JsonMap};
use serde_json::json;

use super::AnthropicFollowupError;
use super::followup_kwargs::prepare_followup_kwargs;
use super::max_tokens::resolve_max_tokens;
use super::patch::{
    AnthropicPatchRequest, FollowupLogging, build_anthropic_request_patch, patch_log_line,
};
use super::plan::{AnthropicFollowup, AnthropicPlanRequest};
use crate::call_transformation::WebSearchTransformation;
use crate::call_transformation::outcome::SearchResult;
use crate::call_transformation::wire::{Call, Response, Thinking};
use crate::chat_responses_followup::ChatResponsesFollowup;
use crate::chat_responses_followup::search::SearchExecutor;
use crate::execution::search::unclassified;
use crate::loop_eligibility::ToolsDict;
use crate::native_blocks::ResponseJson;
use crate::records::{RichWebSearchInput, SearchOutcome};

const MODEL: &str = "claude-sonnet-4-5";
const ONE_MESSAGE: &str = r#"[{"role": "user", "content": "hi"}]"#;
const ONE_CALL: &str = r#"{"content":[{"type":"tool_use","id":"toolu_1","name":"web_search","input":{"query":"first"}}]}"#;
const TWO_CALLS: &str = r#"{"content":[{"type":"tool_use","id":"toolu_1","name":"web_search","input":{"query":"first"}},{"type":"tool_use","id":"toolu_2","name":"web_search","input":{"query":"second"}}]}"#;

#[derive(Clone)]
enum Script {
    Text(String),
    Failure(String),
}

struct ScriptedSearches {
    scripts: Mutex<VecDeque<Script>>,
    queries: Mutex<Vec<String>>,
    rich: Mutex<Vec<Option<RichWebSearchInput>>>,
}

impl ScriptedSearches {
    fn new(scripts: Vec<Script>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(VecDeque::from(scripts)),
            queries: Mutex::new(Vec::new()),
            rich: Mutex::new(Vec::new()),
        })
    }

    fn with_texts(texts: &[&str]) -> Arc<Self> {
        Self::new(
            texts
                .iter()
                .map(|text| Script::Text((*text).to_owned()))
                .collect(),
        )
    }

    fn recorded(&self) -> Vec<String> {
        self.queries.lock().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |queries| queries.clone(),
        )
    }

    fn rich_inputs(&self) -> Vec<Option<RichWebSearchInput>> {
        self.rich.lock().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |inputs| inputs.clone(),
        )
    }
}

#[async_trait]
impl SearchExecutor for ScriptedSearches {
    async fn execute_search(
        &self,
        query: &str,
        _kwargs: &JsonMap,
        rich: Option<RichWebSearchInput>,
    ) -> SearchResult {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(query.to_owned());
        self.rich
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(rich);
        let queued = self
            .scripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(script) = queued else {
            return SearchResult::Failed(unclassified(
                "the case scripts no search response".to_owned(),
            ));
        };
        match script {
            Script::Text(text) => SearchResult::Succeeded {
                text,
                response: None,
            },
            Script::Failure(message) => SearchResult::Failed(unclassified(message)),
        }
    }
}

fn map(text: &str) -> JsonMap {
    serde_json::from_str(text).unwrap()
}

fn message_list(text: &str) -> Vec<FollowupMessage> {
    serde_json::from_str(text).unwrap()
}

fn thinking_list(text: &str) -> Vec<Thinking> {
    serde_json::from_str(text).unwrap()
}

fn call_list(text: &str) -> Vec<Call> {
    let response: Response = serde_json::from_str(text).unwrap();
    let (found, calls) =
        WebSearchTransformation::transform_request(&response, false, "anthropic").unwrap();
    assert!(found);
    calls
}

fn followup(searcher: &Arc<ScriptedSearches>) -> AnthropicFollowup {
    let search: Arc<dyn SearchExecutor> = Arc::clone(searcher);
    AnthropicFollowup {
        followup: ChatResponsesFollowup { search },
    }
}

fn tools_dict(
    tool_calls: Vec<Call>,
    response_format: &'static str,
    thinking_blocks: Option<Vec<ResponseJson>>,
) -> ToolsDict {
    ToolsDict {
        tool_calls,
        tool_type: "websearch",
        provider: "anthropic".to_owned(),
        response_format,
        thinking_blocks,
    }
}

async fn patch_of(
    searcher: &Arc<ScriptedSearches>,
    calls: &[Call],
    optional_params: &JsonMap,
    kwargs: &JsonMap,
) -> (AgenticLoopRequestPatch, Vec<SearchOutcome>) {
    let messages = message_list(ONE_MESSAGE);
    let request = AnthropicPatchRequest {
        model: MODEL,
        messages: &messages,
        tool_calls: calls,
        thinking_blocks: &[],
        optional_params,
        logging: None,
        kwargs,
    };
    build_anthropic_request_patch(searcher.as_ref(), &request)
        .await
        .unwrap()
}

async fn plan_value(
    searcher: &Arc<ScriptedSearches>,
    tools: &ToolsDict,
    kwargs: &JsonMap,
) -> ResponseJson {
    let followup = followup(searcher);
    let messages = message_list(ONE_MESSAGE);
    let optional_params = map("{}");
    let request = AnthropicPlanRequest {
        model: MODEL,
        messages: &messages,
        optional_params: &optional_params,
        logging: None,
        kwargs,
    };
    let plan = followup
        .build_agentic_loop_plan(tools, &request)
        .await
        .unwrap();
    serde_json::to_value(plan).unwrap()
}

#[test]
fn resolve_max_tokens_defaults_to_1024() {
    assert_eq!(resolve_max_tokens(&map("{}"), &map("{}")).unwrap(), 1024);
}

#[test]
fn resolve_max_tokens_prefers_optional_params_over_kwargs() {
    let optional_params = map(r#"{"max_tokens": 2048}"#);
    let kwargs = map(r#"{"max_tokens": 512}"#);
    assert_eq!(resolve_max_tokens(&optional_params, &kwargs).unwrap(), 2048);
}

#[test]
fn resolve_max_tokens_uses_kwargs_when_optional_params_has_none() {
    let kwargs = map(r#"{"max_tokens": 512}"#);
    assert_eq!(resolve_max_tokens(&map("{}"), &kwargs).unwrap(), 512);
}

#[test]
fn resolve_max_tokens_raises_a_max_tokens_at_the_budget() {
    let optional_params = map(r#"{"max_tokens": 2000, "thinking": {"budget_tokens": 2000}}"#);
    assert_eq!(
        resolve_max_tokens(&optional_params, &map("{}")).unwrap(),
        3024
    );
}

#[test]
fn resolve_max_tokens_rounds_the_budget_up() {
    let optional_params = map(r#"{"max_tokens": 1000, "thinking": {"budget_tokens": 1500.5}}"#);
    assert_eq!(
        resolve_max_tokens(&optional_params, &map("{}")).unwrap(),
        2525
    );
}

#[test]
fn resolve_max_tokens_keeps_a_max_tokens_above_the_budget() {
    let optional_params = map(r#"{"max_tokens": 4000, "thinking": {"budget_tokens": 2000}}"#);
    assert_eq!(
        resolve_max_tokens(&optional_params, &map("{}")).unwrap(),
        4000
    );
}

#[test]
fn resolve_max_tokens_ignores_a_zero_or_negative_budget() {
    let zero = map(r#"{"max_tokens": 0, "thinking": {"budget_tokens": 0}}"#);
    assert_eq!(resolve_max_tokens(&zero, &map("{}")).unwrap(), 0);
    let negative = map(r#"{"max_tokens": 1000, "thinking": {"budget_tokens": -5}}"#);
    assert_eq!(resolve_max_tokens(&negative, &map("{}")).unwrap(), 1000);
}

#[test]
fn resolve_max_tokens_rejects_a_fractional_max_tokens() {
    let optional_params = map(r#"{"max_tokens": 1.5}"#);
    let result = resolve_max_tokens(&optional_params, &map("{}"));
    assert!(matches!(
        result,
        Err(AnthropicFollowupError::InvalidMaxTokens(_))
    ));
}

#[test]
fn resolve_max_tokens_rejects_a_non_object_thinking() {
    let optional_params = map(r#"{"thinking": true}"#);
    let result = resolve_max_tokens(&optional_params, &map("{}"));
    assert!(matches!(
        result,
        Err(AnthropicFollowupError::InvalidThinking(_))
    ));
}

#[test]
fn resolve_max_tokens_rejects_a_non_number_budget() {
    let optional_params = map(r#"{"thinking": {"budget_tokens": "big"}}"#);
    let result = resolve_max_tokens(&optional_params, &map("{}"));
    assert!(matches!(
        result,
        Err(AnthropicFollowupError::InvalidBudgetTokens(_))
    ));
}

#[test]
fn resolve_max_tokens_rejects_a_budget_beyond_the_range() {
    let optional_params = map(r#"{"thinking": {"budget_tokens": 1e300}}"#);
    let result = resolve_max_tokens(&optional_params, &map("{}"));
    assert!(matches!(
        result,
        Err(AnthropicFollowupError::BudgetTokensTooLarge)
    ));
}

#[test]
fn followup_kwargs_drops_interception_keys_and_the_logging_object() {
    let kwargs = map(
        r#"{"_websearch_interception_emit_native_blocks": true, "_websearch_interception_converted_stream": false, "litellm_logging_obj": {"x": 1}, "_compression_interception_x": 1, "litellm_call_id": "call-1", "metadata": {"team": "t"}}"#,
    );
    let filtered = prepare_followup_kwargs(&kwargs);
    let keys: Vec<&str> = filtered.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["_compression_interception_x", "litellm_call_id", "metadata"]
    );
}

#[test]
fn followup_kwargs_drops_a_bare_interception_prefix() {
    let filtered = prepare_followup_kwargs(&map(r#"{"_websearch_interception": 1, "other": 2}"#));
    assert_eq!(filtered.len(), 1);
    assert!(filtered.contains_key("other"));
}

#[test]
fn followup_kwargs_keeps_every_other_key() {
    let filtered = prepare_followup_kwargs(&map(r#"{"temperature": 0.2, "metadata": {}}"#));
    assert_eq!(filtered.len(), 2);
}

#[tokio::test]
async fn request_patch_searches_every_call_and_keeps_the_order() {
    let tool_calls = call_list(TWO_CALLS);
    let searcher = ScriptedSearches::new(vec![
        Script::Text("one".to_owned()),
        Script::Failure("boom".to_owned()),
    ]);
    let (patch, outcomes) = patch_of(&searcher, &tool_calls, &map("{}"), &map("{}")).await;
    assert_eq!(searcher.recorded(), ["first", "second"]);
    assert!(matches!(
        outcomes.first(),
        Some(SearchOutcome::Succeeded(_))
    ));
    assert!(matches!(outcomes.get(1), Some(SearchOutcome::Failed(_))));
    let value = serde_json::to_value(&patch).unwrap();
    assert_eq!(
        value.pointer("/messages/2/content/0/content"),
        Some(&json!("one"))
    );
    assert_eq!(
        value.pointer("/messages/2/content/1/content"),
        Some(&json!("Search failed: boom"))
    );
    assert_eq!(
        value.pointer("/messages/2/content/1/tool_use_id"),
        Some(&json!("toolu_2"))
    );
    assert_eq!(patch.max_tokens, Some(1024));
    assert_eq!(patch.model.as_deref(), Some(MODEL));
}

#[tokio::test]
async fn request_patch_uses_the_empty_result_for_a_missing_query() {
    let tool_calls = call_list(
        r#"{"content":[{"type":"tool_use","id":"toolu_1","name":"web_search","input":{}}]}"#,
    );
    let searcher = ScriptedSearches::new(Vec::new());
    let (patch, outcomes) = patch_of(&searcher, &tool_calls, &map("{}"), &map("{}")).await;
    assert!(searcher.recorded().is_empty());
    assert_eq!(outcomes.len(), 1);
    let value = serde_json::to_value(&patch).unwrap();
    assert_eq!(
        value.pointer("/messages/2/content/0/content"),
        Some(&json!("No search query provided"))
    );
    assert_eq!(value.pointer("/messages/2/role"), Some(&json!("user")));
}

#[tokio::test]
async fn request_patch_prepends_thinking_blocks_to_the_assistant_message() {
    let tool_calls = call_list(ONE_CALL);
    let thinking_blocks = thinking_list(
        r#"[{"type":"thinking","thinking":"hmm","signature":"sig"},{"type":"redacted_thinking","data":"zzz"}]"#,
    );
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let messages = message_list(ONE_MESSAGE);
    let optional_params = map("{}");
    let kwargs = map("{}");
    let request = AnthropicPatchRequest {
        model: MODEL,
        messages: &messages,
        tool_calls: &tool_calls,
        thinking_blocks: &thinking_blocks,
        optional_params: &optional_params,
        logging: None,
        kwargs: &kwargs,
    };
    let (patch, _outcomes) = build_anthropic_request_patch(searcher.as_ref(), &request)
        .await
        .unwrap();
    let value = serde_json::to_value(&patch).unwrap();
    assert_eq!(
        value.pointer("/messages/1/content/0/type"),
        Some(&json!("thinking"))
    );
    assert_eq!(
        value.pointer("/messages/1/content/1/type"),
        Some(&json!("redacted_thinking"))
    );
    assert_eq!(
        value.pointer("/messages/1/content/2/type"),
        Some(&json!("tool_use"))
    );
}

#[tokio::test]
async fn request_patch_takes_the_model_from_the_logging_object() {
    let tool_calls = call_list(ONE_CALL);
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let messages = message_list(ONE_MESSAGE);
    let optional_params = map("{}");
    let kwargs = map("{}");
    let details = map(r#"{"agentic_loop_params": {"model": "claude-unprefixed"}}"#);
    let logging = FollowupLogging {
        call_id: Some("call-abc"),
        model_call_details: &details,
    };
    let request = AnthropicPatchRequest {
        model: MODEL,
        messages: &messages,
        tool_calls: &tool_calls,
        thinking_blocks: &[],
        optional_params: &optional_params,
        logging: Some(&logging),
        kwargs: &kwargs,
    };
    let (patch, _outcomes) = build_anthropic_request_patch(searcher.as_ref(), &request)
        .await
        .unwrap();
    assert_eq!(patch.model.as_deref(), Some("claude-unprefixed"));
}

#[tokio::test]
async fn request_patch_falls_back_to_the_model_argument() {
    let tool_calls = call_list(ONE_CALL);
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let messages = message_list(ONE_MESSAGE);
    let optional_params = map("{}");
    let kwargs = map("{}");
    let details = map(r#"{"agentic_loop_params": {}}"#);
    let logging = FollowupLogging {
        call_id: Some("call-abc"),
        model_call_details: &details,
    };
    let request = AnthropicPatchRequest {
        model: MODEL,
        messages: &messages,
        tool_calls: &tool_calls,
        thinking_blocks: &[],
        optional_params: &optional_params,
        logging: Some(&logging),
        kwargs: &kwargs,
    };
    let (patch, _outcomes) = build_anthropic_request_patch(searcher.as_ref(), &request)
        .await
        .unwrap();
    assert_eq!(patch.model.as_deref(), Some(MODEL));
}

#[tokio::test]
async fn request_patch_strips_max_tokens_from_optional_params() {
    let tool_calls = call_list(ONE_CALL);
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let optional_params = map(r#"{"max_tokens": 700, "temperature": 0.2}"#);
    let (patch, _outcomes) = patch_of(&searcher, &tool_calls, &optional_params, &map("{}")).await;
    assert_eq!(patch.max_tokens, Some(700));
    let value = serde_json::to_value(&patch).unwrap();
    assert_eq!(value.pointer("/optional_params/max_tokens"), None);
    assert_eq!(
        value.pointer("/optional_params/temperature"),
        Some(&json!(0.2_f64))
    );
}

#[tokio::test]
async fn request_patch_drops_the_interception_kwargs() {
    let tool_calls = call_list(ONE_CALL);
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let kwargs = map(
        r#"{"_websearch_interception_emit_native_blocks": true, "litellm_logging_obj": {}, "litellm_call_id": "call-9"}"#,
    );
    let (patch, _outcomes) = patch_of(&searcher, &tool_calls, &map("{}"), &kwargs).await;
    assert_eq!(patch.kwargs.len(), 1);
    assert!(patch.kwargs.contains_key("litellm_call_id"));
}

#[test]
fn request_patch_logs_the_logging_call_id_over_the_kwargs_one() {
    let messages = message_list(ONE_MESSAGE);
    let tool_calls = call_list(ONE_CALL);
    let optional_params = map("{}");
    let kwargs = map(r#"{"litellm_call_id": "call-9"}"#);
    let details = map("{}");
    let logging = FollowupLogging {
        call_id: Some("call-abc"),
        model_call_details: &details,
    };
    let request = AnthropicPatchRequest {
        model: MODEL,
        messages: &messages,
        tool_calls: &tool_calls,
        thinking_blocks: &[],
        optional_params: &optional_params,
        logging: Some(&logging),
        kwargs: &kwargs,
    };
    assert_eq!(
        patch_log_line(&request, MODEL, 3, 2),
        "WebSearchInterception: Built anthropic request patch [call_id=call-abc model=claude-sonnet-4-5 messages=3 searches=2]"
    );
}

#[test]
fn request_patch_logs_the_kwargs_call_id_without_a_logging_object() {
    let messages = message_list(ONE_MESSAGE);
    let tool_calls = call_list(ONE_CALL);
    let optional_params = map("{}");
    let kwargs = map(r#"{"litellm_call_id": "call-9"}"#);
    let request = AnthropicPatchRequest {
        model: MODEL,
        messages: &messages,
        tool_calls: &tool_calls,
        thinking_blocks: &[],
        optional_params: &optional_params,
        logging: None,
        kwargs: &kwargs,
    };
    assert_eq!(
        patch_log_line(&request, MODEL, 4, 1),
        "WebSearchInterception: Built anthropic request patch [call_id=call-9 model=claude-sonnet-4-5 messages=4 searches=1]"
    );
}

#[test]
fn request_patch_logs_an_unknown_call_id_without_any_source() {
    let messages = message_list(ONE_MESSAGE);
    let tool_calls = call_list(ONE_CALL);
    let optional_params = map("{}");
    let kwargs = map("{}");
    let request = AnthropicPatchRequest {
        model: MODEL,
        messages: &messages,
        tool_calls: &tool_calls,
        thinking_blocks: &[],
        optional_params: &optional_params,
        logging: None,
        kwargs: &kwargs,
    };
    assert_eq!(
        patch_log_line(&request, MODEL, 2, 0),
        "WebSearchInterception: Built anthropic request patch [call_id=unknown model=claude-sonnet-4-5 messages=2 searches=0]"
    );
}

#[tokio::test]
async fn request_patch_appends_the_assistant_and_user_messages() {
    let tool_calls = call_list(ONE_CALL);
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let messages =
        message_list(r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"sure"}]"#);
    let optional_params = map("{}");
    let kwargs = map("{}");
    let request = AnthropicPatchRequest {
        model: MODEL,
        messages: &messages,
        tool_calls: &tool_calls,
        thinking_blocks: &[],
        optional_params: &optional_params,
        logging: None,
        kwargs: &kwargs,
    };
    let (patch, _outcomes) = build_anthropic_request_patch(searcher.as_ref(), &request)
        .await
        .unwrap();
    assert_eq!(patch.messages.as_ref().map(Vec::len), Some(4));
    let value = serde_json::to_value(&patch).unwrap();
    assert_eq!(value.pointer("/messages/0/content"), Some(&json!("hi")));
    assert_eq!(value.pointer("/messages/2/role"), Some(&json!("assistant")));
}

#[tokio::test]
async fn request_patch_passes_the_rich_search_input() {
    let tool_calls = call_list(
        r#"{"content":[{"type":"tool_use","id":"toolu_1","name":"web_search","input":{"query":"q","objective":"find","search_queries":["a","b"]}}]}"#,
    );
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let (patch, _outcomes) = patch_of(&searcher, &tool_calls, &map("{}"), &map("{}")).await;
    assert_eq!(
        searcher.rich_inputs(),
        [Some(RichWebSearchInput {
            objective: Some("find".to_owned()),
            search_queries: Some(vec!["a".to_owned(), "b".to_owned()]),
        })]
    );
    assert_eq!(patch.messages.as_ref().map(Vec::len), Some(3));
}

#[tokio::test]
async fn anthropic_plan_returns_the_patch_and_metadata() {
    let searcher = ScriptedSearches::with_texts(&["one", "two"]);
    let tools = tools_dict(call_list(TWO_CALLS), "anthropic", None);
    let value = plan_value(&searcher, &tools, &map("{}")).await;
    assert_eq!(value.pointer("/run_agentic_loop"), Some(&json!(true)));
    assert_eq!(value.pointer("/terminate"), Some(&json!(false)));
    assert_eq!(
        value.pointer("/metadata/tool_type"),
        Some(&json!("websearch"))
    );
    assert_eq!(
        value.pointer("/metadata/response_format"),
        Some(&json!("anthropic"))
    );
    assert_eq!(value.pointer("/metadata/websearch_native_blocks"), None);
    assert_eq!(
        value.pointer("/request_patch/messages/2/role"),
        Some(&json!("user"))
    );
}

#[tokio::test]
async fn anthropic_plan_prepends_thinking_blocks_from_the_tools() {
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let blocks: Vec<ResponseJson> =
        serde_json::from_str(r#"[{"type":"thinking","thinking":"hmm","signature":"sig"}]"#)
            .unwrap();
    let tools = tools_dict(call_list(ONE_CALL), "anthropic", Some(blocks));
    let value = plan_value(&searcher, &tools, &map("{}")).await;
    assert_eq!(
        value.pointer("/request_patch/messages/1/content/0/type"),
        Some(&json!("thinking"))
    );
}

#[tokio::test]
async fn anthropic_plan_carries_native_blocks_for_mixed_searches() {
    let searcher = ScriptedSearches::new(vec![
        Script::Text("one".to_owned()),
        Script::Failure("boom".to_owned()),
    ]);
    let tools = tools_dict(call_list(TWO_CALLS), "anthropic", None);
    let kwargs = map(r#"{"_websearch_interception_emit_native_blocks": true}"#);
    let value = plan_value(&searcher, &tools, &kwargs).await;
    assert_eq!(value.pointer("/run_agentic_loop"), Some(&json!(true)));
    assert_eq!(
        value.pointer("/metadata/websearch_native_blocks/0/type"),
        Some(&json!("server_tool_use"))
    );
    assert_eq!(
        value.pointer("/metadata/websearch_native_blocks/1/type"),
        Some(&json!("web_search_tool_result"))
    );
    assert_eq!(
        value.pointer("/metadata/websearch_native_blocks/2/type"),
        Some(&json!("server_tool_use"))
    );
    assert_eq!(
        value.pointer("/metadata/websearch_native_blocks/3/content/type"),
        Some(&json!("web_search_tool_result_error"))
    );
    assert_eq!(
        value.pointer("/metadata/websearch_native_blocks/3/content/error_code"),
        Some(&json!("unavailable"))
    );
}

#[tokio::test]
async fn anthropic_plan_terminates_when_every_native_search_failed() {
    let searcher = ScriptedSearches::new(vec![
        Script::Failure("a".to_owned()),
        Script::Failure("b".to_owned()),
    ]);
    let tools = tools_dict(call_list(TWO_CALLS), "anthropic", None);
    let kwargs = map(r#"{"_websearch_interception_emit_native_blocks": true}"#);
    let value = plan_value(&searcher, &tools, &kwargs).await;
    assert_eq!(value.pointer("/run_agentic_loop"), Some(&json!(false)));
    assert_eq!(value.pointer("/terminate"), Some(&json!(true)));
    assert_eq!(
        value.pointer("/stop_reason"),
        Some(&json!("web_search_failed"))
    );
    assert_eq!(value.pointer("/request_patch"), Some(&json!(null)));
    assert_eq!(
        value.pointer("/metadata/websearch_native_blocks/1/content/error_code"),
        Some(&json!("unavailable"))
    );
}

#[tokio::test]
async fn anthropic_plan_keeps_running_when_every_plain_search_failed() {
    let searcher = ScriptedSearches::new(vec![
        Script::Failure("a".to_owned()),
        Script::Failure("b".to_owned()),
    ]);
    let tools = tools_dict(call_list(TWO_CALLS), "anthropic", None);
    let value = plan_value(&searcher, &tools, &map("{}")).await;
    assert_eq!(value.pointer("/run_agentic_loop"), Some(&json!(true)));
    assert_eq!(value.pointer("/terminate"), Some(&json!(false)));
    assert_eq!(value.pointer("/metadata/websearch_native_blocks"), None);
    assert_eq!(
        value.pointer("/request_patch/messages/2/content/0/content"),
        Some(&json!("Search failed: a"))
    );
}

#[tokio::test]
async fn anthropic_plan_keeps_running_with_no_searches() {
    let searcher = ScriptedSearches::new(Vec::new());
    let tools = tools_dict(Vec::new(), "anthropic", None);
    let kwargs = map(r#"{"_websearch_interception_emit_native_blocks": true}"#);
    let value = plan_value(&searcher, &tools, &kwargs).await;
    assert_eq!(value.pointer("/run_agentic_loop"), Some(&json!(true)));
    assert_eq!(
        value.pointer("/metadata/websearch_native_blocks"),
        Some(&json!([]))
    );
}

#[tokio::test]
async fn anthropic_plan_rejects_a_non_boolean_native_flag() {
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let tools = tools_dict(call_list(ONE_CALL), "anthropic", None);
    let followup = followup(&searcher);
    let messages = message_list(ONE_MESSAGE);
    let optional_params = map("{}");
    let kwargs = map(r#"{"_websearch_interception_emit_native_blocks": "yes"}"#);
    let request = AnthropicPlanRequest {
        model: MODEL,
        messages: &messages,
        optional_params: &optional_params,
        logging: None,
        kwargs: &kwargs,
    };
    let result = followup.build_agentic_loop_plan(&tools, &request).await;
    assert!(matches!(
        result,
        Err(AnthropicFollowupError::InvalidNativeBlocksFlag(_))
    ));
}

#[tokio::test]
async fn plan_dispatch_sends_chat_completions_to_the_chat_builder() {
    let searcher = ScriptedSearches::with_texts(&["one", "two"]);
    let tools = tools_dict(call_list(TWO_CALLS), "openai", None);
    let kwargs = map(r#"{"_agentic_loop_api_surface": "chat_completions"}"#);
    let value = plan_value(&searcher, &tools, &kwargs).await;
    assert_eq!(
        value.pointer("/metadata/response_format"),
        Some(&json!("openai"))
    );
    assert_eq!(
        value.pointer("/request_patch/messages/2/role"),
        Some(&json!("tool"))
    );
}

#[tokio::test]
async fn plan_dispatch_sends_responses_to_the_responses_builder() {
    let searcher = ScriptedSearches::with_texts(&["one", "two"]);
    let tools = tools_dict(call_list(TWO_CALLS), "responses", None);
    let kwargs = map(r#"{"_agentic_loop_api_surface": "responses"}"#);
    let value = plan_value(&searcher, &tools, &kwargs).await;
    assert_eq!(
        value.pointer("/metadata/response_format"),
        Some(&json!("responses"))
    );
    assert_eq!(
        value.pointer("/request_patch/messages/1/type"),
        Some(&json!("function_call"))
    );
}

#[tokio::test]
async fn plan_dispatch_sends_an_unknown_surface_to_the_anthropic_path() {
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let tools = tools_dict(call_list(ONE_CALL), "openai", None);
    let kwargs = map(r#"{"_agentic_loop_api_surface": "legacy"}"#);
    let value = plan_value(&searcher, &tools, &kwargs).await;
    assert_eq!(
        value.pointer("/metadata/response_format"),
        Some(&json!("anthropic"))
    );
    assert_eq!(
        value.pointer("/request_patch/messages/2/role"),
        Some(&json!("user"))
    );
}

#[tokio::test]
async fn plan_dispatch_sends_a_missing_surface_to_the_anthropic_path() {
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let tools = tools_dict(call_list(ONE_CALL), "openai", None);
    let value = plan_value(&searcher, &tools, &map("{}")).await;
    assert_eq!(
        value.pointer("/metadata/response_format"),
        Some(&json!("anthropic"))
    );
}

#[tokio::test]
async fn request_patch_rejects_a_non_object_agentic_loop_params() {
    let tool_calls = call_list(ONE_CALL);
    let searcher = ScriptedSearches::with_texts(&["one"]);
    let messages = message_list(ONE_MESSAGE);
    let optional_params = map("{}");
    let kwargs = map("{}");
    let details = map(r#"{"agentic_loop_params": 5}"#);
    let logging = FollowupLogging {
        call_id: Some("call-abc"),
        model_call_details: &details,
    };
    let request = AnthropicPatchRequest {
        model: MODEL,
        messages: &messages,
        tool_calls: &tool_calls,
        thinking_blocks: &[],
        optional_params: &optional_params,
        logging: Some(&logging),
        kwargs: &kwargs,
    };
    let result = build_anthropic_request_patch(searcher.as_ref(), &request).await;
    assert!(matches!(
        result,
        Err(AnthropicFollowupError::InvalidAgenticLoopParams(_))
    ));
}

#[tokio::test]
async fn anthropic_plan_rejects_a_malformed_thinking_block() {
    let searcher = ScriptedSearches::new(Vec::new());
    let blocks: Vec<ResponseJson> = vec![json!({"type": "thinking", "thinking": "hmm"})];
    let tools = tools_dict(Vec::new(), "anthropic", Some(blocks));
    let followup = followup(&searcher);
    let messages = message_list(ONE_MESSAGE);
    let optional_params = map("{}");
    let kwargs = map("{}");
    let request = AnthropicPlanRequest {
        model: MODEL,
        messages: &messages,
        optional_params: &optional_params,
        logging: None,
        kwargs: &kwargs,
    };
    let result = followup.build_agentic_loop_plan(&tools, &request).await;
    assert!(matches!(
        result,
        Err(AnthropicFollowupError::InvalidThinkingBlock(_))
    ));
}
