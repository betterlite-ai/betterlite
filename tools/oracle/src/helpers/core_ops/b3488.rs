use std::time::Duration;

use betterlite_core::api::genai_http::error::GenAiHttpError;
use betterlite_core::api::genai_http::service::{
    AsyncGenerateContentOutcome, GenerateContentCall, GenerateContentDeps, GenerateContentOutcome,
    StreamSettings, async_generate_content, generate_content,
};
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::genai_setup::params::GenerateContentLiteLLMParams;
use betterlite_models::catalog::BundledCatalog;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_vertex::google_native_content::response::GenerateContentResponse;
use chrono::DateTime;
use indexmap::IndexMap;
use serde::Deserialize;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::config::{ConfigError, NativeConfig};
use self::logging::{Sink, logging_state, pre_post_env};
use self::report::{Run, helper_error};
use self::wire::Source;

mod config;
mod logging;
mod report;
mod wire;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    mode: Mode,
    provider: String,
    model: String,
    contents: Json,
    config: Object,
    #[serde(default)]
    tools: Option<Json>,
    #[serde(default)]
    system_instruction: Option<Json>,
    litellm_params: Params,
    #[serde(default)]
    extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    extra_body: Option<Object>,
    #[serde(default)]
    timeout: Option<f64>,
    stream: bool,
    reply: ReplyInput,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Sync,
    Async,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Params {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    model_info: Option<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    status: u16,
    #[serde(default)]
    headers: IndexMap<String, String>,
    body: String,
}

type SyncResult =
    Result<GenerateContentOutcome<GenerateContentResponse>, GenAiHttpError<ConfigError>>;
type AsyncResult =
    Result<AsyncGenerateContentOutcome<GenerateContentResponse>, GenAiHttpError<ConfigError>>;

enum Outcome {
    Sync(SyncResult),
    Async(AsyncResult),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(call(input))
    }
}

async fn call(input: &Input) -> HelperResult {
    let params = params_of(input)?;
    let mut bundled = BundledCatalog::default();
    let config = NativeConfig::new(bundled.get());
    let source = Source::new(input)?;
    let request = request_of(input, &params, timeout_of(input)?);
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let sink = Sink;
    let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").or_value_error()?;
    let env = pre_post_env(&redactor, &sink, now);
    let mut state = logging_state(input, &redactor)?;
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let outcome = match input.mode {
        Mode::Sync => Outcome::Sync(generate_content(&deps, &request, &mut log).await),
        Mode::Async => Outcome::Async(async_generate_content(&deps, &request, &mut log).await),
    };
    let run = Run {
        source: &source,
        state: &state,
    };
    match outcome {
        Outcome::Sync(result) => sync_report(result, &run),
        Outcome::Async(result) => async_report(result, &run).await,
    }
}

fn sync_report(outcome: SyncResult, run: &Run<'_>) -> HelperResult {
    match outcome {
        Ok(GenerateContentOutcome::Response(response)) => run.unary(&response),
        Ok(GenerateContentOutcome::Stream(stream)) => run.sync_stream(*stream),
        Err(error) => Err(helper_error(error)),
    }
}

async fn async_report(outcome: AsyncResult, run: &Run<'_>) -> HelperResult {
    match outcome {
        Ok(AsyncGenerateContentOutcome::Response(response)) => run.unary(&response),
        Ok(AsyncGenerateContentOutcome::Stream(stream)) => run.async_stream(*stream).await,
        Err(error) => Err(helper_error(error)),
    }
}

fn request_of<'call>(
    input: &'call Input,
    params: &'call GenerateContentLiteLLMParams,
    timeout: Option<Duration>,
) -> GenerateContentCall<'call> {
    GenerateContentCall {
        model: &input.model,
        contents: &input.contents,
        config: &input.config,
        tools: input.tools.as_ref(),
        custom_llm_provider: &input.provider,
        litellm_params: params,
        extra_headers: input.extra_headers.as_ref(),
        extra_body: input.extra_body.as_ref(),
        system_instruction: input.system_instruction.as_ref(),
        timeout,
        stream: input.stream,
    }
}

// GenericLiteLLMParams carries api_key, api_base and model_info; the rest reaches the handler through extra
fn params_of(input: &Input) -> Result<GenerateContentLiteLLMParams, Box<HelperError>> {
    let params = &input.litellm_params;
    let mut kwargs = Object::new();
    if let Some(key) = &params.api_key {
        kwargs.insert("api_key".to_owned(), Json::Text(key.clone()));
    }
    if let Some(base) = &params.api_base {
        kwargs.insert("api_base".to_owned(), Json::Text(base.clone()));
    }
    if let Some(info) = &params.model_info {
        kwargs.insert("model_info".to_owned(), Json::Object(info.clone()));
    }
    GenerateContentLiteLLMParams::from_kwargs(&kwargs).or_value_error()
}

fn timeout_of(input: &Input) -> Result<Option<Duration>, Box<HelperError>> {
    input
        .timeout
        .map(Duration::try_from_secs_f64)
        .transpose()
        .or_value_error()
}
