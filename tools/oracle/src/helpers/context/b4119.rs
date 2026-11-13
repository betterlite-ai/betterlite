use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_websearch::call_transformation::outcome::SearchResult;
use betterlite_websearch::execution::search::unclassified;
use betterlite_websearch::call_transformation::wire::{
    Call, CallFunction, CallKind, Input as ToolInput,
};
use betterlite_websearch::chat_responses_followup::ChatResponsesFollowup;
use betterlite_websearch::chat_responses_followup::chat::FollowupRequest;
use betterlite_websearch::chat_responses_followup::error::FollowupError;
use betterlite_websearch::chat_responses_followup::rerun::ChatRerunRequest;
use betterlite_websearch::chat_responses_followup::responses::{
    ResponsesInput, ResponsesRequest,
};
use betterlite_websearch::chat_responses_followup::search::SearchExecutor;
use betterlite_websearch::records::RichWebSearchInput;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Builder;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    BuildChatPlan,
    BuildResponsesPlan,
    BuildChatRerun,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    model: String,
    #[serde(default)]
    messages: Vec<FollowupMessage>,
    input: Option<ResponsesInput>,
    response_format: Option<String>,
    tool_calls: Vec<ToolCall>,
    #[serde(default)]
    optional_params: JsonMap,
    #[serde(default)]
    kwargs: JsonMap,
    #[serde(default)]
    searches: Vec<SearchScript>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCall {
    id: Option<String>,
    #[serde(rename = "type")]
    kind: CallKind,
    name: String,
    #[serde(default)]
    input: ToolInput,
    function: Option<ToolFunction>,
    arguments: Option<String>,
    call_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolFunction {
    name: String,
    #[serde(default)]
    arguments: ToolInput,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchScript {
    kind: ScriptKind,
    #[serde(default)]
    text: String,
    #[serde(default)]
    message: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScriptKind {
    Success,
    Error,
}

#[derive(Debug, PartialEq, Serialize)]
struct RecordedSearch {
    query: String,
    rich: Option<RichWebSearchInput>,
}

struct ScriptedSearch {
    scripts: Mutex<VecDeque<SearchScript>>,
    searches: Mutex<Vec<RecordedSearch>>,
}

impl ScriptedSearch {
    fn new(scripts: &[SearchScript]) -> Self {
        Self {
            scripts: Mutex::new(scripts.iter().cloned().collect()),
            searches: Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<RecordedSearch> {
        match self.searches.lock() {
            Ok(mut searches) => std::mem::take(&mut *searches),
            Err(poisoned) => std::mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[async_trait]
impl SearchExecutor for ScriptedSearch {
    async fn execute_search(
        &self,
        query: &str,
        _kwargs: &JsonMap,
        rich: Option<RichWebSearchInput>,
    ) -> SearchResult {
        if let Ok(mut searches) = self.searches.lock() {
            searches.push(RecordedSearch {
                query: query.to_owned(),
                rich,
            });
        }
        let mut scripts = self.scripts.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(script) = scripts.pop_front() else {
            return SearchResult::Failed(unclassified(
                "the case scripts no search response".to_owned(),
            ));
        };
        match script.kind {
            ScriptKind::Success => SearchResult::Succeeded {
                text: script.text,
                response: None,
            },
            ScriptKind::Error => SearchResult::Failed(unclassified(script.message)),
        }
    }
}

impl ToolCall {
    fn to_call(&self) -> Call {
        let call_id = match self.kind {
            CallKind::FunctionCall => Some(self.call_id.clone()),
            CallKind::ToolUse | CallKind::Function => None,
        };
        Call {
            id: self.id.clone(),
            kind: self.kind,
            name: self.name.clone(),
            input: self.input.clone(),
            function: self.function.as_ref().map(|function| CallFunction {
                name: function.name.clone(),
                arguments: function.arguments.clone(),
            }),
            arguments: self.arguments.clone(),
            call_id,
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let searcher = Arc::new(ScriptedSearch::new(&input.searches));
        let calls: Vec<Call> = input.tool_calls.iter().map(ToolCall::to_call).collect();
        let shared = Arc::clone(&searcher);
        let search: Arc<dyn SearchExecutor> = shared;
        let followup = ChatResponsesFollowup { search };
        let runtime = Builder::new_current_thread().build().map_err(failure)?;
        let result = runtime
            .block_on(async { run_operation(&followup, input, &calls).await })
            .map_err(failure)?;
        Ok(json!({"result": result, "searches": searcher.recorded()}))
    }
}

async fn run_operation(
    followup: &ChatResponsesFollowup,
    input: &Input,
    calls: &[Call],
) -> Result<Value, FollowupError> {
    match input.operation {
        Operation::BuildChatPlan => {
            let format = input.response_format.as_deref().unwrap_or("openai");
            let request = chat_request(input);
            let plan = followup.build_chat_plan(calls, format, &request).await?;
            serde_json::to_value(plan).map_err(FollowupError::from)
        }
        Operation::BuildResponsesPlan => {
            let responses = input
                .input
                .clone()
                .unwrap_or_else(|| ResponsesInput::Items(Vec::new()));
            let request = ResponsesRequest {
                model: &input.model,
                input: &responses,
                optional_params: &input.optional_params,
                kwargs: &input.kwargs,
            };
            let plan = followup.build_responses_plan(calls, &request).await?;
            serde_json::to_value(plan).map_err(FollowupError::from)
        }
        Operation::BuildChatRerun => {
            let format = input.response_format.as_deref().unwrap_or("openai");
            let request = chat_request(input);
            let rerun = followup.build_chat_rerun(calls, format, &request).await?;
            rerun_value(&rerun)
        }
    }
}

fn chat_request(input: &Input) -> FollowupRequest<'_> {
    FollowupRequest {
        model: &input.model,
        messages: &input.messages,
        optional_params: &input.optional_params,
        kwargs: &input.kwargs,
    }
}

fn rerun_value(rerun: &ChatRerunRequest) -> Result<Value, FollowupError> {
    let mut value = serde_json::Map::new();
    value.insert("model".to_owned(), json!(rerun.model));
    value.insert("messages".to_owned(), serde_json::to_value(&rerun.messages)?);
    for (key, raw) in &rerun.params {
        value.insert(key.clone(), serde_json::from_str(raw.get())?);
    }
    for (key, raw) in &rerun.kwargs {
        value.insert(key.clone(), serde_json::from_str(raw.get())?);
    }
    Ok(Value::Object(value))
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "SearchPlanError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
