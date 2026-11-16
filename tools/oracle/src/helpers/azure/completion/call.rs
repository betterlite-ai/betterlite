use std::future::Future;
use std::sync::Mutex;

use betterlite_azure::completion::arguments::{CompletionCall, Runtime};
use betterlite_azure::completion::config::AzureOpenAITextConfig;
use betterlite_azure::completion::error::AzureTextError;
use betterlite_azure::completion::handler::{TextCompletionOutcome, completion as run_completion};
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
use super::source::{ScriptedSource, ScriptedTransport};
use super::view;

pub(super) mod stream;

const CLOCK: i64 = 1_700_000_000;
const STREAM_TIMESTAMP: f64 = 1_700_000_000.0;
static IDENTIFIER: fn() -> Uuid = || Uuid::from_u128(1);
static CLOCK_READ: fn() -> i64 = || CLOCK;

// the prompt factory and the clock the handler reads instead of the process clock
pub(super) struct Fixed {
    factory: PromptFactory,
    now: DateTime<Utc>,
}

impl Default for Fixed {
    fn default() -> Self {
        Self {
            factory: PromptFactory {
                cache: Mutex::new(builtin_entries()),
                hub: Hub::Unreachable,
            },
            now: DateTime::from_timestamp(CLOCK, 0).unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
        }
    }
}

// the key, token and headers a call borrows for its whole run
pub(super) struct Secrets {
    key: Option<SecretString>,
    token: Option<SecretString>,
    headers: Option<SecretHeaders>,
}

impl Secrets {
    pub(super) fn of(case: &CallCase) -> Self {
        Self {
            key: case.api_key.clone().map(SecretString::from),
            token: case.azure_ad_token.clone().map(SecretString::from),
            headers: case.headers.clone().map(SecretHeaders),
        }
    }
}

pub(super) fn runtime(fixed: &Fixed) -> Runtime<'_> {
    Runtime {
        prompts: &fixed.factory,
        now: fixed.now,
        identifier: &IDENTIFIER,
        clock: &CLOCK_READ,
        stream_now: || StreamInstant {
            seconds: CLOCK,
            timestamp: STREAM_TIMESTAMP,
        },
        stream_id: || Uuid::from_u128(2),
    }
}

pub(super) fn block_on<Fut>(future: Fut) -> Result<Fut::Output, Box<HelperError>>
where
    Fut: Future,
{
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    Ok(runtime.block_on(future))
}

// the SDK client the fixture builds: the case may ask for the other kind than acompletion selects
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
    let base_url = case.api_base.as_deref().map_or_else(
        || "https://unit.test/openai/".to_owned(),
        |base| format!("{}/openai/", base.trim_end_matches('/')),
    );
    ScriptedSource::new(
        kind,
        key.map(SecretString::from),
        &base_url,
        case.answer.clone(),
    )
}

pub(super) fn build_call<'case>(
    case: &'case CallCase,
    secrets: &'case Secrets,
    params: &'case mut Extra,
    source: &ScriptedSource,
) -> CompletionCall<'case, ScriptedTransport> {
    CompletionCall {
        model: case.model.as_deref(),
        messages: case.messages.as_deref(),
        api_key: secrets.key.as_ref(),
        api_base: case.api_base.as_deref(),
        api_version: case.api_version.as_deref(),
        azure_ad_token: secrets.token.as_ref(),
        azure_ad_token_provider: None,
        timeout: case.timeout,
        optional_params: params,
        litellm_params: case.litellm_params.clone(),
        cloudflare_params: CloudflareParams::default(),
        acompletion: case.acompletion,
        headers: secrets.headers.as_ref(),
        supplied: Some(source.handle()),
    }
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

pub(super) fn stream_report(
    source: &ScriptedSource,
    recorder: &Recorder,
    mut stream: CompletionStream,
) -> Result<Json, Box<HelperError>> {
    let chunks = block_on(async move {
        let mut values = Vec::new();
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|error| view::failure(&error))?;
            values.push(view::chunk_value(&chunk));
        }
        Ok::<Vec<Json>, Box<HelperError>>(values)
    })??;
    Ok(view::report(
        source,
        recorder,
        view::Outcome {
            name: "stream",
            reply: Json::Null,
            chunks: Json::List(chunks),
            error: Json::Null,
        },
    ))
}

pub(super) fn completion(case: &CallCase) -> Result<Json, Box<HelperError>> {
    let source = source(case);
    let secrets = Secrets::of(case);
    let mut params = case.optional_params.clone();
    let mut call = build_call(case, &secrets, &mut params, &source);
    let fixed = Fixed::default();
    let mut recorder = Recorder::default();
    let outcome = block_on(run_completion(
        &source,
        &mut call,
        &runtime(&fixed),
        &mut recorder,
    ))?;
    match outcome {
        Ok(TextCompletionOutcome::Buffered(reply)) => Ok(view::report(
            &source,
            &recorder,
            view::Outcome {
                name: "buffered",
                reply: view::reply_value(&reply),
                chunks: Json::Null,
                error: Json::Null,
            },
        )),
        Ok(TextCompletionOutcome::Stream(stream)) => stream_report(&source, &recorder, stream),
        Err(error) => Ok(error_report(&source, &recorder, &error)),
    }
}

pub(super) fn auth(case: &CallCase) -> Json {
    let secrets = Secrets::of(case);
    let headers = validate_environment(secrets.key.as_ref(), secrets.token.as_ref());
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
