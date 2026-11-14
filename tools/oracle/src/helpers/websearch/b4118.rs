use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_websearch::anthropic_followup::patch::FollowupLogging;
use betterlite_websearch::anthropic_followup::plan::{AnthropicFollowup, AnthropicPlanRequest};
use betterlite_websearch::call_transformation::outcome::{SearchFailure, SearchResult};
use betterlite_websearch::call_transformation::wire::{
    Call, CallFunction, CallKind, Input as ToolInput,
};
use betterlite_websearch::chat_responses_followup::ChatResponsesFollowup;
use betterlite_websearch::chat_responses_followup::search::SearchExecutor;
use betterlite_websearch::execution::search::unclassified;
use betterlite_websearch::loop_eligibility::ToolsDict;
use betterlite_websearch::records::RichWebSearchInput;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    #[serde(default)]
    model: String,
    #[serde(default)]
    messages: Vec<FollowupMessage>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
    #[serde(default)]
    thinking_blocks: Vec<Value>,
    #[serde(default)]
    optional_params: JsonMap,
    #[serde(default)]
    kwargs: JsonMap,
    #[serde(default)]
    logging: Option<LoggingCase>,
    #[serde(default)]
    searches: Vec<SearchScript>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoggingCase {
    #[serde(default)]
    model_call_details: JsonMap,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchScript {
    kind: ScriptKind,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScriptKind {
    Success,
    RateLimit,
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

struct ScriptedSearch {
    scripts: Mutex<VecDeque<SearchScript>>,
    queries: Mutex<Vec<String>>,
}

impl ScriptedSearch {
    fn new(scripts: &[SearchScript]) -> Self {
        Self {
            scripts: Mutex::new(VecDeque::from(scripts.to_vec())),
            queries: Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<String> {
        match self.queries.lock() {
            Ok(mut queries) => std::mem::take(&mut *queries),
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
        _rich: Option<RichWebSearchInput>,
    ) -> SearchResult {
        if let Ok(mut queries) = self.queries.lock() {
            queries.push(query.to_owned());
        }
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
        match script.kind {
            ScriptKind::Success => SearchResult::Succeeded {
                text: script.text.unwrap_or_default(),
                response: None,
            },
            ScriptKind::RateLimit => {
                SearchResult::Failed(rate_limit(&script.message.unwrap_or_default()))
            }
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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let results = input
            .cases
            .iter()
            .map(run_case)
            .collect::<Result<Vec<Value>, Box<crate::helpers::HelperError>>>()?;
        Ok(json!({"results": results}))
    }
}

fn run_case(case: &Case) -> Result<Value, Box<crate::helpers::HelperError>> {
    let (executor, logging) = searches(case);
    let tools = tools_dict(case);
    let search: Arc<dyn SearchExecutor> = Arc::clone(&executor);
    let followup = AnthropicFollowup {
        followup: ChatResponsesFollowup { search },
    };
    let request = plan_request(case, logging.as_ref());
    let plan = block_on(followup.build_agentic_loop_plan(&tools, &request))?.map_err(failure)?;
    let value = serde_json::to_value(plan).map_err(failure)?;
    Ok(json!({"plan": value, "searches": executor.recorded()}))
}

fn tools_dict(case: &Case) -> ToolsDict {
    ToolsDict {
        tool_calls: case.tool_calls.iter().map(ToolCall::to_call).collect(),
        tool_type: "websearch",
        provider: "anthropic".to_owned(),
        response_format: "anthropic",
        thinking_blocks: Some(case.thinking_blocks.clone()),
    }
}

fn plan_request<'request>(
    case: &'request Case,
    logging: Option<&'request FollowupLogging<'request>>,
) -> AnthropicPlanRequest<'request> {
    AnthropicPlanRequest {
        model: &case.model,
        messages: &case.messages,
        optional_params: &case.optional_params,
        logging,
        kwargs: &case.kwargs,
    }
}

fn searches(case: &Case) -> (Arc<ScriptedSearch>, Option<FollowupLogging<'_>>) {
    let executor = Arc::new(ScriptedSearch::new(&case.searches));
    let logging = case.logging.as_ref().map(|spec| FollowupLogging {
        call_id: Some("call-1"),
        model_call_details: &spec.model_call_details,
    });
    (executor, logging)
}

fn block_on<Future>(future: Future) -> Result<Future::Output, Box<crate::helpers::HelperError>>
where
    Future: std::future::Future,
{
    Builder::new_current_thread()
        .build()
        .map_err(failure)?
        .block_on(future)
}

fn rate_limit(message: &str) -> SearchFailure {
    SearchFailure {
        class: ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context: ErrorContext {
                message: message.to_owned(),
                model: None,
                provider: None,
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            },
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        message: message.to_owned(),
    }
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "AnthropicFollowupError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
