use std::future::Future;
use std::sync::Mutex;

use betterlite_azure::completion::arguments::{CompletionCall, Runtime};
use betterlite_azure::completion::config::AzureOpenAITextConfig;
use betterlite_azure::completion::error::AzureTextError;
use betterlite_azure::completion::handler::{AzureTextCompletion, TextCompletionOutcome};
use betterlite_azure::completion::request::validate_environment;
use betterlite_azure::endpoints::cloudflare::CloudflareParams;
use betterlite_azure::endpoints::projection::ClientKind;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use secrecy::SecretString;
use tokio::runtime::Builder;
use uuid::Uuid;

use super::CallCase;
use super::hooks::Recorder;
use super::source::ScriptedSource;
use super::view;

mod stream;

const CLOCK: i64 = 1_700_000_000;

// the clocks and ids the handler reads instead of the process clock
pub(super) struct Fixed {
    factory: PromptFactory,
    now: DateTime<Utc>,
    identifier: Box<dyn Fn() -> Uuid>,
    clock: Box<dyn Fn() -> i64>,
    stream_now: fn() -> StreamInstant,
    stream_id: fn() -> Uuid,
}

impl Fixed {
    pub(super) fn new() -> Self {
        Self {
            factory: PromptFactory {
                cache: Mutex::new(builtin_entries()),
                hub: Hub::Unreachable,
            },
            now: DateTime::from_timestamp(CLOCK, 0).unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
            identifier: Box::new(|| Uuid::from_u128(1)),
            clock: Box::new(|| CLOCK),
            stream_now: || StreamInstant {
                seconds: CLOCK,
                timestamp: CLOCK as f64,
            },
            stream_id: || Uuid::from_u128(2),
        }
    }
}

pub(super) fn runtime(fixed: &Fixed) -> Runtime<'_> {
    Runtime {
        prompts: &fixed.factory,
        now: fixed.now,
        identifier: fixed.identifier.as_ref(),
        clock: fixed.clock.as_ref(),
        stream_now: fixed.stream_now,
        stream_id: fixed.stream_id,
    }
}

pub(super) fn block_on<Fut: Future>(future: Fut) -> Result<Fut::Output, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    Ok(runtime.block_on(future))
}

pub(super) fn collect(stream: CompletionStream) -> Result<Vec<Json>, Box<HelperError>> {
    let values: Result<Vec<Json>, Box<HelperError>> = block_on(async move {
        let mut stream = stream;
        let mut values = Vec::new();
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|error| view::failure(&error))?;
            values.push(view::chunk_value(&chunk));
        }
        Ok(values)
    })?;
    values
}

// the SDK client the fixture builds: the case may ask for the other kind to reach the isinstance guard
pub(super) fn source(case: &CallCase) -> ScriptedSource {
    let asynchronous = match case.client.as_deref() {
        Some("sync") => false,
        Some("async") => true,
        Some(_other) | None => case.acompletion,
    };
    let kind = if asynchronous {
        ClientKind::AsyncAzureOpenAi
    } else {
        ClientKind::AzureOpenAi
    };
    let key = case
        .client_api_key
        .as_ref()
        .or(case.api_key.as_ref())
        .cloned()
        .or_else(|| {
            case.azure_ad_token
                .as_ref()
                .map(|_token| "<missing API key>".to_owned())
        });
    let base_url = match case.api_base.as_deref() {
        Some(base) => format!("{}/openai/", base.trim_end_matches('/')),
        None => "https://unit.test/openai/".to_owned(),
    };
    ScriptedSource::new(
        kind,
        key.map(SecretString::from),
        &base_url,
        case.answer.clone(),
    )
}

pub(super) fn error_report(
    source: &ScriptedSource,
    recorder: &Recorder,
    error: &AzureTextError,
) -> Json {
    view::report(
        source,
        recorder,
        view::Outcome {
            name: "error",
            reply: Json::Null,
            chunks: Json::Null,
            error: view::error_value(error),
        },
    )
}

pub(super) fn completion(case: &CallCase) -> Result<Json, Box<HelperError>> {
    let source = source(case);
    let handle = source.handle();
    let mut params = case.optional_params.clone();
    let headers = case.headers.clone().map(SecretHeaders);
    let call = CompletionCall {
        model: case.model.as_deref(),
        messages: case.messages.as_deref(),
        api_key: case
            .api_key
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        api_base: case.api_base.as_deref(),
        api_version: case.api_version.as_deref(),
        azure_ad_token: case
            .azure_ad_token
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        azure_ad_token_provider: None,
        timeout: case.timeout,
        optional_params: &mut params,
        litellm_params: case.litellm_params.clone(),
        cloudflare_params: CloudflareParams::default(),
        acompletion: case.acompletion,
        headers: headers.as_ref(),
        supplied: Some(handle),
    };
    let fixed = Fixed::new();
    let mut recorder = Recorder::default();
    let handler = AzureTextCompletion {
        source: source.clone(),
    };
    let outcome = match block_on(handler.completion(call, &runtime(&fixed), &mut recorder))? {
        Ok(outcome) => outcome,
        Err(error) => return Ok(error_report(&source, &recorder, &error)),
    };
    let value = match outcome {
        TextCompletionOutcome::Buffered(reply) => view::report(
            &source,
            &recorder,
            view::Outcome {
                name: "buffered",
                reply: view::reply_value(&reply),
                chunks: Json::Null,
                error: Json::Null,
            },
        ),
        TextCompletionOutcome::Stream(stream) => {
            let chunks = collect(stream)?;
            view::report(
                &source,
                &recorder,
                view::Outcome {
                    name: "stream",
                    reply: Json::Null,
                    chunks: Json::List(chunks),
                    error: Json::Null,
                },
            )
        }
    };
    Ok(value)
}

pub(super) fn auth(case: &CallCase) -> Json {
    let key = case
        .api_key
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    let token = case
        .azure_ad_token
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    let headers = validate_environment(key.as_ref(), token.as_ref());
    let fields: Extra = headers
        .0
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    let mut result = Extra::new();
    result.insert("headers".to_owned(), Json::Object(fields));
    Json::Object(result)
}

pub(super) fn config(constructor: Option<&Extra>) -> Result<Json, Box<HelperError>> {
    let raw = Json::Object(constructor.cloned().unwrap_or_default());
    let raw_value = serde_json::to_value(raw).or_value_error()?;
    let config: AzureOpenAITextConfig = serde_json::from_value(raw_value).or_value_error()?;
    let entries = [
        ("frequency_penalty", config.frequency_penalty),
        ("logit_bias", config.logit_bias),
        ("max_tokens", config.max_tokens),
        ("n", config.n),
        ("presence_penalty", config.presence_penalty),
        ("stop", config.stop),
        ("temperature", config.temperature),
        ("top_p", config.top_p),
    ];
    let mut fields = Extra::new();
    for (name, setting) in entries {
        fields.insert(name.to_owned(), setting.unwrap_or(Json::Null));
    }
    let mut result = Extra::new();
    result.insert("fields".to_owned(), Json::Object(fields));
    Ok(Json::Object(result))
}
