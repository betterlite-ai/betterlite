use std::convert::Infallible;

use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::params::{DEFAULT_MAX_TOKENS, ParamsRequest};
use betterlite_anthropic::request::{AnthropicRequest, BillingHeader, PreparedRequest};
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{RawResponse, ResponseRequest};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_vertex::vertex_claude_chat::params::map_openai_params as map_vertex_params;
use betterlite_vertex::vertex_claude_chat::request::prepare as prepare_vertex_request;
use betterlite_vertex::vertex_claude_chat::response::{TransformCall, transform_response};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

mod fixture;
mod tests;

const PROVIDER: &str = "vertex_ai";

type Environment<'env> = CapabilityEnvironment<'env, NoDynamicMetadata, fixture::NoHfTransport>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    fixture: fixture::FixtureInput,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    MapOpenaiParams(MapStep),
    TransformRequest(RequestStep),
    TransformResponse(ResponseStep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapStep {
    model: String,
    non_default_params: IndexMap<String, Json>,
    optional_params: IndexMap<String, Json>,
    drop_params: bool,
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestStep {
    model: String,
    messages: Vec<Json>,
    optional_params: IndexMap<String, Json>,
    headers: IndexMap<String, String>,
    drop_params: bool,
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseStep {
    model: String,
    status: u16,
    raw_text: String,
    headers: IndexMap<String, String>,
    messages: Vec<Json>,
    optional_params: IndexMap<String, Json>,
    litellm_params: IndexMap<String, Json>,
    request_data: IndexMap<String, Json>,
    json_mode: Option<bool>,
}

struct StepContext<'context> {
    budgets: &'context Settings,
    catalogs: &'context CatalogSources<'context>,
}

struct FixedCounter;

impl ReasoningTokenCounter for FixedCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = fixture::Fixture::load(&input.fixture).or_config_error()?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        runtime.block_on(run_steps(input, &fixture))
    }
}

async fn run_steps(input: &Input, fixture: &fixture::Fixture) -> HelperResult {
    let resolution = fixture.resolution();
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let catalogs = fixture.catalogs();
    let budgets = Settings::default();
    let context = StepContext {
        budgets: &budgets,
        catalogs: &catalogs,
    };
    let mut answers = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
        let step_answer = match step {
            Step::MapOpenaiParams(step) => map_answer(step, &context, &mut environment).await?,
            Step::TransformRequest(step) => {
                request_answer(step, &context, &mut environment).await?
            }
            Step::TransformResponse(step) => response_answer(step)?,
        };
        answers.push(step_answer);
    }
    Ok(Value::Array(answers))
}

async fn map_answer(
    step: &MapStep,
    context: &StepContext<'_>,
    environment: &mut Environment<'_>,
) -> Result<Value, Box<HelperError>> {
    let request = ParamsRequest {
        model: &step.model,
        provider: PROVIDER,
        drop: DropParams {
            global: step.global_drop_params,
            call: step.drop_params,
        },
        budgets: context.budgets,
        default_max_tokens: DEFAULT_MAX_TOKENS,
        catalogs: context.catalogs,
    };
    let mut mapped = step.optional_params.clone();
    map_vertex_params(&request, &step.non_default_params, &mut mapped, environment)
        .await
        .or_value_error()?;
    Ok(json!({"optional_params": mapped}))
}

async fn request_answer(
    step: &RequestStep,
    context: &StepContext<'_>,
    environment: &mut Environment<'_>,
) -> Result<Value, Box<HelperError>> {
    let defaults = OptionDefaults::default();
    let catalog = ModelCatalog::parse(b"{}", UnknownFields::Warn).or_config_error()?;
    let mut litellm_params = IndexMap::new();
    if step.drop_params {
        litellm_params.insert("drop_params".to_owned(), Json::Bool(true));
    }
    let request = AnthropicRequest {
        model: &step.model,
        custom_llm_provider: None,
        messages: step.messages.clone(),
        optional_params: step.optional_params.clone(),
        litellm_params,
        headers: AnthropicHeaders {
            values: step.headers.clone(),
        },
        defaults: &defaults,
        catalog: &catalog,
        default_chat_max_tokens: DEFAULT_MAX_TOKENS,
        billing_header: BillingHeader::Keep,
        modify_params: false,
        global_drop_params: step.global_drop_params,
    };
    let fetcher = fixture::RefusingFetcher;
    let blocks = fixture::NoFileBlocks;
    let new_id = || "generated-id".to_owned();
    let prompts = PromptDeps {
        fetcher: &fetcher,
        blocks: &blocks,
        new_id: &new_id,
    };
    let prepared = prepare_vertex_request(request, environment, context.catalogs, &prompts)
        .await
        .or_value_error()?;
    Ok(body_answer(prepared))
}

// Both sides report the beta membership sorted, so the comparison never depends on Python's hash order.
fn body_answer(prepared: PreparedRequest) -> Value {
    let mut body = prepared.body;
    let mut headers = prepared.headers.values;
    let betas = sorted_members(body.shift_remove("anthropic_beta"));
    let beta_header = sorted_members(headers.shift_remove("anthropic-beta").map(Json::Text));
    json!({"body": body, "anthropic_beta": betas, "beta_header": beta_header})
}

fn sorted_members(value: Option<Json>) -> Vec<String> {
    let mut members: Vec<String> = match value {
        Some(Json::List(items)) => items
            .into_iter()
            .filter_map(|item| match item {
                Json::Text(text) => Some(text),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
        Some(Json::Text(text)) => text
            .split(',')
            .filter(|member| !member.is_empty())
            .map(str::to_owned)
            .collect(),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => Vec::new(),
    };
    members.sort();
    members
}

fn response_answer(step: &ResponseStep) -> Result<Value, Box<HelperError>> {
    let headers = fixture::header_map(&step.headers)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let sink = fixture::Silent;
    let mut state = fixture::logging_state(&redactor)?;
    let env = fixture::logging_env(&redactor, &sink)?;
    let request = ResponseRequest {
        messages: &step.messages,
        optional_params: &step.optional_params,
        litellm_params: &step.litellm_params,
        request_data: &step.request_data,
        json_mode: step.json_mode,
        now: 0,
        disable_add_prefix_to_prompt: false,
    };
    let raw = RawResponse {
        status: step.status,
        headers: &headers,
        body: step.raw_text.as_bytes(),
    };
    let counter = FixedCounter;
    let mut envelope = ResponseEnvelope::new("chatcmpl-b1333".to_owned());
    let call = TransformCall {
        model: &step.model,
        envelope: &mut envelope,
        raw: &raw,
        request: &request,
        state: &mut state,
        env: &env,
        counter: &counter,
    };
    transform_response(call).or_value_error()?;
    Ok(json!({"model": envelope.model}))
}
