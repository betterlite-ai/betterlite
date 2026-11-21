use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::compression_interception_state::Settings;
use betterlite_context::agentic_callback::AgenticLogging;
use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_core::api::compression_interception::engine::{
    CompressionCall, CompressionEngine, CompressionOutput,
};
use betterlite_core::api::compression_interception::hook::CompressionInterception;
use betterlite_core::api::compression_retrieval_loop::CompressionRetrievalLoop as _;
use betterlite_core::api::compression_retrieval_loop::plan::RetrievalLoopRequest;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use thiserror::Error;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    enabled: Option<bool>,
    response: Option<Json>,
    tools: Option<Json>,
    seed: Option<Seed>,
    tool_calls: Option<Vec<Json>>,
    thinking_blocks: Option<Vec<Json>>,
    messages: Option<Vec<Json>>,
    model: String,
    optional_params: Option<IndexMap<String, Json>>,
    kwargs: Option<IndexMap<String, Json>>,
    logging: Option<LoggingInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Gate,
    Plan,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Seed {
    call_id: String,
    cache: IndexMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoggingInput {
    call_id: String,
    agentic_model: Option<String>,
}

#[derive(Debug, Error)]
#[error("the retrieval loop helper never compresses")]
pub(crate) struct Unused;

pub(crate) struct IdleEngine;

#[async_trait]
impl CompressionEngine for IdleEngine {
    type Failure = Unused;

    async fn compress(&self, _call: &CompressionCall<'_>) -> Result<CompressionOutput, Unused> {
        Err(Unused)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input.function {
            Function::Gate => gate_value(input),
            Function::Plan => plan_value(input),
        }
    }
}

fn gate_value(input: &Input) -> HelperResult {
    let interception = interception(input.enabled.unwrap_or(true));
    let response = input.response.clone().unwrap_or(Json::Null);
    let (run, ready) = interception.should_run_agentic_loop(&response, input.tools.as_ref());
    let mut output = IndexMap::with_capacity(2);
    output.insert("run".to_owned(), Json::Bool(run));
    output.insert("tools".to_owned(), Json::Object(ready));
    serde_json::to_value(&output).or_value_error()
}

fn plan_value(input: &Input) -> HelperResult {
    let interception = interception(true);
    if let Some(seed) = input.seed.as_ref() {
        interception
            .cache
            .store(seed.call_id.clone(), seed.cache.clone());
    }
    let tools = loop_tools(input);
    let optional_params = input.optional_params.clone().unwrap_or_default();
    let kwargs = input.kwargs.clone().unwrap_or_default();
    let messages = input.messages.clone().unwrap_or_default();
    let logging = logging_value(input)?;
    let request = RetrievalLoopRequest {
        model: input.model.as_str(),
        messages: &messages,
        optional_params: &optional_params,
        logging: logging.as_ref(),
        kwargs: &kwargs,
    };
    let plan = interception
        .build_agentic_loop_plan(&tools, &request)
        .or_value_error()?;
    serde_json::to_value(&plan).or_value_error()
}

fn loop_tools(input: &Input) -> IndexMap<String, Json> {
    let mut tools = IndexMap::with_capacity(2);
    let tool_calls = input.tool_calls.clone().unwrap_or_default();
    tools.insert("tool_calls".to_owned(), Json::List(tool_calls));
    let thinking_blocks = input.thinking_blocks.clone().unwrap_or_default();
    tools.insert("thinking_blocks".to_owned(), Json::List(thinking_blocks));
    tools
}

fn logging_value(input: &Input) -> Result<Option<AgenticLogging>, Box<HelperError>> {
    let Some(spec) = input.logging.as_ref() else {
        return Ok(None);
    };
    let agentic_loop_params = spec
        .agentic_model
        .as_deref()
        .map(agentic_params)
        .transpose()?;
    Ok(Some(AgenticLogging {
        call_id: Some(spec.call_id.clone()),
        agentic_loop_params,
    }))
}

fn agentic_params(model: &str) -> Result<JsonMap, Box<HelperError>> {
    let mut params = JsonMap::with_capacity(1);
    params.insert("model".to_owned(), to_raw_value(model).or_value_error()?);
    Ok(params)
}

fn interception(enabled: bool) -> CompressionInterception<IdleEngine> {
    let settings = Settings {
        enabled,
        compression_trigger: 200_000,
        compression_target: None,
        embedding_model: None,
        embedding_model_params: None,
    };
    CompressionInterception::with_clock(settings, Arc::new(IdleEngine), Arc::new(|| 0.0_f64))
}

mod tests;
