mod logging;
mod world;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_core::api::genai::generate_content::error::GenerateContentFailure;
use betterlite_core::api::genai::generate_content::stream::generate_content_stream;
use betterlite_core::api::genai::generate_content::unary::generate_content;
use betterlite_core::api::genai::generate_content::{
    GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest,
    mock_generate_content_response,
};
use betterlite_core::api::genai_http::stream::StreamSettings;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::schema::SchemaLimits;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::catalog::ModelCatalog;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::logging::logging_state;
use self::world::{Config, ConfigError, Executor, ExecutorError, Source, World};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    entry: Entry,
    model: String,
    contents: Json,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    mock_response: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Entry {
    Mock,
    GenerateContent,
    GenerateContentStream,
}

type Failure = GenerateContentFailure<ConfigError, ExecutorError>;

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
    let kwargs = kwargs_of(input);
    let request = request_of(input, &kwargs);
    let world = World {
        catalog: ModelCatalog::default(),
        names: BTreeSet::from([String::from("gemini"), String::from("openai")]),
        environment: BTreeMap::new(),
        config: Config,
        source: Source,
        executor: Executor,
    };
    let capabilities = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    run_entry(&world, &capabilities, &request, input).await
}

async fn run_entry(
    world: &World,
    capabilities: &CatalogCapabilities,
    request: &GenerateContentRequest<'_>,
    input: &Input,
) -> HelperResult {
    let resolution = world.resolution();
    let configs = world.configs();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let deps = GenerateContentDeps {
        configs: &configs,
        resolution: &resolution,
        capabilities,
        limits: SchemaLimits::default(),
        source: &world.source,
        supplied_client: None,
        executor: &world.executor,
        stream: StreamSettings::default(),
        request_timeout: request_timeout(),
        redactor: &redactor,
    };
    let mut state = logging_state(&redactor)?;
    let sink = Sink;
    let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").or_value_error()?;
    let env = pre_post_env(&redactor, &sink, now);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let outcome = match input.entry {
        Entry::Mock => {
            let answer = mock_generate_content_response(input.mock_response.as_deref());
            return serde_json::to_value(answer).or_value_error();
        }
        Entry::GenerateContent => generate_content(&deps, request, &mut log).await,
        Entry::GenerateContentStream => generate_content_stream(&deps, request, &mut log).await,
    };
    answer_of(outcome)
}

fn kwargs_of(input: &Input) -> Object {
    let mut kwargs = Object::new();
    if let Some(text) = &input.mock_response {
        kwargs.insert("mock_response".to_owned(), Json::Text(text.clone()));
    }
    kwargs
}

fn request_of<'call>(input: &'call Input, kwargs: &'call Object) -> GenerateContentRequest<'call> {
    GenerateContentRequest {
        model: &input.model,
        contents: &input.contents,
        config: None,
        tools: None,
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        timeout: None,
        custom_llm_provider: input.custom_llm_provider.as_deref(),
        kwargs,
    }
}

// REQUEST_TIMEOUT's default: the cases never reach a provider, so only its presence matters here
const fn request_timeout() -> RequestTimeout {
    RequestTimeout {
        seconds: 6000.0_f64,
        explicitly_set: false,
    }
}

// Compare strips the class and the message; the status is the one exception_type maps the failure to
fn answer_of(outcome: Result<GenerateContentAnswer<Json>, Failure>) -> HelperResult {
    match outcome {
        Ok(GenerateContentAnswer::Mock(value)) => serde_json::to_value(value).or_value_error(),
        Ok(
            GenerateContentAnswer::Adapter(_)
            | GenerateContentAnswer::Native(_)
            | GenerateContentAnswer::AsyncNative(_),
        ) => Err(unmodelled()),
        Err(failure) => Err(Box::new(HelperError {
            class: "ApiConnection".to_owned(),
            status: Some(failure.class.metadata().status_code),
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: failure.to_string(),
        })),
    }
}

fn unmodelled() -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: "the oracle call answered without the caller's mock".to_owned(),
    })
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

fn pre_post_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}
