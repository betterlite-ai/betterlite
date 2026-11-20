use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::chat_call::CancellationToken;
use betterlite_azure::chat_call::arguments::{ChatCall, Runtime};
use betterlite_azure::chat_call::client::ClientHandle;
use betterlite_azure::chat_call::error::AzureChatError;
use betterlite_azure::chat_call::handler::{self, ChatOutcome};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::cloudflare::CloudflareParams;
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_config::sections::openai_chat_content::Settings as ChatContentSettings;
use betterlite_openai::output_limits::tokens::{PromptCountError, PromptTokens};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Object;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use tokio::runtime::Builder;
use uuid::Uuid;

use super::CallCase;
use super::hooks::Recorder;
use super::source::{ScriptedSource, ScriptedTransport, kind_of};
use super::view::{self, Outcome};

pub(crate) mod os_series;

const CLOCK: i64 = 1_700_000_000;

static IDENTIFIER: fn() -> Uuid = || Uuid::from_u128(7);
static CLOCK_READ: fn() -> i64 = || CLOCK;
static RECOVERED_CLOCK: fn() -> u64 = || 1_700_000_000;
static NOW: fn() -> Duration = || Duration::from_secs(1);
static STREAM_NOW: fn() -> StreamInstant = || StreamInstant {
    seconds: CLOCK,
    timestamp: 1_700_000_000.0,
};

struct FixedTokens;

#[async_trait]
impl PromptTokens for FixedTokens {
    async fn count(&self, _model: &str, _messages: &[Value]) -> Result<usize, PromptCountError> {
        Ok(11)
    }
}

struct NoCredentials;

fn unscripted() -> SourceFailure {
    SourceFailure {
        class: "KeyError".to_owned(),
        status: None,
        message: "the owned chat path never resolves a credential".to_owned(),
    }
}

impl CredentialSources for NoCredentials {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unscripted())
    }
}

fn block_on<Fut>(future: Fut) -> Fut::Output
where
    Fut: Future,
{
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("oracle runtime");
    runtime.block_on(future)
}

fn message_text(secret: Option<&SecretString>) -> Value {
    secret.map_or(Value::Null, |value| {
        Value::String(value.expose_secret().to_owned())
    })
}

fn optional_object(case: &CallCase) -> Result<Object, Box<HelperError>> {
    let value = serde_json::to_value(&case.optional_params).or_value_error()?;
    serde_json::from_value(value).or_value_error()
}

fn litellm_params(case: &CallCase) -> LiteLlmParams {
    case.litellm_params.clone()
}

fn runtime<'runtime>(tokens: &'runtime FixedTokens) -> Runtime<'runtime> {
    Runtime {
        identifier: IDENTIFIER,
        clock: CLOCK_READ,
        recovered_clock: RECOVERED_CLOCK,
        now: NOW,
        stream_now: STREAM_NOW,
        prompt_tokens: tokens,
    }
}

fn source_of(case: &CallCase) -> ScriptedSource {
    let kind = kind_of(case.client.as_deref(), case.acompletion);
    let key = case
        .client_api_key
        .as_ref()
        .or(case.api_key.as_ref())
        .cloned();
    ScriptedSource::new(kind, key.map(SecretString::from), case.answer.clone())
}

// the settings the owned completion reads instead of the process configuration
struct Settings {
    azure: AzureSettings,
    chat: ChatContentSettings,
    sources: NoCredentials,
    cancel: Option<CancellationToken>,
}

fn chat_call<'call, 'settings>(
    case: &'call CallCase,
    secrets: &'call Secrets,
    params: &'call mut Object,
    supplied: Option<ClientHandle<ScriptedTransport>>,
    settings: &'settings Settings,
) -> ChatCall<'call, ScriptedTransport>
where
    'settings: 'call,
{
    ChatCall {
        model: case.model.as_deref(),
        messages: case.messages.as_deref(),
        api_key: secrets.key.as_ref(),
        api_base: &case.api_base,
        api_version: &case.api_version,
        azure_ad_token: secrets.token.as_ref(),
        azure_ad_token_provider: None,
        dynamic_params: false,
        timeout: case.timeout,
        optional_params: params,
        litellm_params: litellm_params(case),
        base_model: case.base_model.as_deref(),
        cloudflare_params: CloudflareParams::default(),
        acompletion: case.acompletion,
        headers: secrets.headers.as_ref(),
        supplied,
        sources: &settings.sources,
        azure: &settings.azure,
        chat: &settings.chat,
        cancel: settings.cancel.as_ref(),
    }
}

fn outcome_of(outcome: Result<ChatOutcome, AzureChatError>) -> Outcome {
    match outcome {
        Ok(ChatOutcome::Buffered(reply)) => Outcome {
            name: "buffered",
            reply: view::reply_value(&reply),
            error: Value::Null,
            forward: Value::Null,
        },
        Ok(ChatOutcome::Stream(_stream)) => Outcome {
            name: "stream",
            reply: Value::Null,
            error: Value::Null,
            forward: Value::Null,
        },
        Err(AzureChatError::Cancelled) => Outcome {
            name: "cancelled",
            reply: Value::Null,
            error: Value::Null,
            forward: Value::Null,
        },
        Err(error) => Outcome {
            name: "error",
            reply: Value::Null,
            error: view::error_value(&error),
            forward: Value::Null,
        },
    }
}

pub(super) fn completion(case: &CallCase) -> Result<Value, Box<HelperError>> {
    let source = source_of(case);
    let secrets = Secrets::of(case);
    let mut params = optional_object(case)?;
    let settings = Settings {
        azure: AzureSettings {
            default_max_retries: case.default_max_retries,
            azure_default_api_version: case.api_version.clone(),
        },
        chat: ChatContentSettings {
            openai_system_messages_first: case.openai_system_messages_first,
        },
        sources: NoCredentials,
        cancel: case.cancel.then(CancellationToken::new),
    };
    if let Some(token) = &settings.cancel {
        token.cancel();
    }
    let supplied = case.supplied.then(|| source.handle());
    let mut call = chat_call(case, &secrets, &mut params, supplied, &settings);
    let tokens = FixedTokens;
    let mut recorder = Recorder::default();
    let outcome = block_on(handler::completion(
        &source,
        &mut call,
        &runtime(&tokens),
        &mut recorder,
    ));
    Ok(view::report(&source, &recorder, &outcome_of(outcome)))
}

struct Secrets {
    key: Option<SecretString>,
    token: Option<SecretString>,
    headers: Option<SecretHeaders>,
}

impl Secrets {
    fn of(case: &CallCase) -> Self {
        Self {
            key: case.api_key.clone().map(SecretString::from),
            token: case.azure_ad_token.clone().map(SecretString::from),
            headers: case.headers.clone().map(SecretHeaders),
        }
    }
}
