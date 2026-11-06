use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::chatgpt::chat::normalizer::ChatGptToolCallNormalizer;
use betterlite_openai::chatgpt::chat::{Authenticator, ChatGptConfig, ChatGptError};
use betterlite_openai::chatgpt::headers::PlatformEnvironment;
use betterlite_openai::chatgpt::headers::session::{LitellmParamsInput, SessionIdentity};
use betterlite_openai::chatgpt::login::error::LoginError;
use betterlite_openai::chatgpt::login::http::{AuthPost, AuthResponse, AuthTransport, HttpFailure};
use betterlite_openai::chatgpt::login::{ChatGptLogin, DeviceDisplay, Pause};
use betterlite_openai::chatgpt::token_store::codec::AuthRecord;
use betterlite_openai::chatgpt::token_store::files::{FileAccess, StoreEvent};
use betterlite_openai::chatgpt::token_store::{ChatGptTokenStore, Clock, StoreError};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::stream::chunk::Chunk;
use futures_util::{StreamExt as _, stream};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

const SESSION_ID: &str = "00000000-0000-4000-8000-000000000000";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    platform: PlatformInput,
    #[serde(default)]
    record: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    now: Decimal,
    step: Step,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformInput {
    system: String,
    release: String,
    machine: String,
}

impl PlatformEnvironment for PlatformInput {
    fn os_type(&self) -> String {
        self.system.clone()
    }

    fn os_release(&self) -> String {
        self.release.clone()
    }

    fn arch(&self) -> String {
        self.machine.clone()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    ProviderInfo(ProviderInfoInput),
    Headers(HeadersInput),
    Params(ParamsInput),
    Normalize(NormalizeInput),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderInfoInput {
    model: String,
    provider: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadersInput {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    headers: Headers,
    #[serde(default)]
    litellm_params: Object,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParamsInput {
    model: String,
    #[serde(default)]
    chat_models: BTreeSet<String>,
    #[serde(default)]
    text_models: BTreeSet<String>,
    requests: Vec<Object>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizeInput {
    chunks: Vec<Value>,
}

struct FixedSession;

impl SessionIdentity for FixedSession {
    fn generate(&self) -> String {
        SESSION_ID.to_owned()
    }
}

struct MemoryFiles {
    content: Option<Vec<u8>>,
}

impl FileAccess for MemoryFiles {
    fn ensure_dir(&mut self, _path: &Path) -> io::Result<()> {
        Ok(())
    }

    fn read(&mut self, _path: &Path) -> io::Result<Vec<u8>> {
        self.content
            .clone()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn write(&mut self, _path: &Path, _record: &AuthRecord) -> io::Result<()> {
        Ok(())
    }

    fn report(&mut self, _event: StoreEvent) {}
}

struct FixedClock {
    now: Decimal,
}

impl Clock for FixedClock {
    fn now(&self) -> Decimal {
        self.now
    }

    fn sleep(&mut self, _duration: Duration) {}
}

impl Pause for FixedClock {
    async fn pause(&mut self, _duration: Duration) {}
}

struct NoTransport;

impl AuthTransport for NoTransport {
    fn post(
        &mut self,
        _request: AuthPost<'_>,
    ) -> impl Future<Output = Result<AuthResponse, HttpFailure>> + Send {
        ready(Err(HttpFailure::Transport(TransportError::Closed)))
    }
}

struct NoDisplay;

impl DeviceDisplay for NoDisplay {
    fn show(&mut self, _verify_url: &str, _user_code: &SecretString) {}
}

struct Recording<L> {
    login: L,
    calls: Mutex<Vec<&'static str>>,
}

impl<L: Authenticator + Send> Authenticator for Recording<L> {
    fn api_base(&self) -> &str {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("get_api_base");
        self.login.api_base()
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, LoginError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("get_access_token");
        self.login.access_token()
    }

    fn account_id(&mut self) -> Option<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("get_account_id");
        self.login.account_id()
    }
}

type Login = ChatGptLogin<MemoryFiles, FixedClock, NoTransport, NoDisplay>;

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("fixture configuration failed")]
    Config(#[from] ConfigError),
    #[error("fixture token store failed")]
    Store(#[from] StoreError),
    #[error("fixture JSON failed")]
    Json(#[from] serde_json::Error),
    #[error("fixture runtime failed")]
    Runtime(#[from] io::Error),
    #[error("ChatGPT preparation failed")]
    Chat(#[from] ChatGptError),
}

fn assemble(
    input: &Input,
    api_key: Option<SecretString>,
    headers: Headers,
) -> Result<ChatGptConfig<Recording<Login>>, FixtureError> {
    let sections = ConfigLoader {
        lookup: input.environment.clone(),
    }
    .load("")?;
    let store = ChatGptTokenStore::new(
        sections.sections.chatgpt_token_store,
        MemoryFiles {
            content: input.record.clone().map(String::into_bytes),
        },
        FixedClock { now: input.now },
    )?;
    Ok(ChatGptConfig {
        base: OpenAiConfig {
            family: ConfigFamily::Other,
            api_key,
            headers,
            settings: OpenAiSettings::default(),
        },
        authenticator: Recording {
            login: ChatGptLogin {
                store,
                http: NoTransport,
                display: NoDisplay,
            },
            calls: Mutex::new(Vec::new()),
        },
        header_settings: sections.sections.chatgpt_headers,
    })
}

fn decode_chunks(chunks: &[Value]) -> Result<Vec<Chunk>, FixtureError> {
    chunks
        .iter()
        .map(|value| serde_json::from_value(value.clone()).map_err(FixtureError::Json))
        .collect()
}

async fn collect_async(chunks: Vec<Chunk>) -> Vec<Chunk> {
    let mut source = ChatGptToolCallNormalizer::from(stream::iter(chunks));
    let mut output = Vec::new();
    while let Some(chunk) = source.next().await {
        output.push(chunk);
    }
    output
}

fn project(chunks: &[Chunk]) -> Result<Value, FixtureError> {
    let values = chunks
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(values))
}

fn provider_value(
    config: &mut ChatGptConfig<Recording<Login>>,
    spec: &ProviderInfoInput,
) -> Result<Object, FixtureError> {
    let api_base = config.authenticator.api_base().to_owned();
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let info = runtime.block_on(config.provider_info(
        &spec.model,
        spec.api_base.as_deref(),
        spec.api_key.as_deref(),
        &spec.provider,
    ))?;
    let mut output = Object::new();
    output.insert(String::from("api_base"), Value::String(api_base));
    output.insert(
        String::from("provider_api_base"),
        Value::String(info.api_base),
    );
    output.insert(
        String::from("api_key"),
        Value::String(info.api_key.expose_secret().to_owned()),
    );
    output.insert(String::from("provider"), Value::String(info.provider));
    Ok(output)
}

fn headers_value(
    config: &mut ChatGptConfig<Recording<Login>>,
    platform: &dyn PlatformEnvironment,
    spec: &HeadersInput,
) -> Object {
    let params = LitellmParamsInput::Mapping(spec.litellm_params.clone());
    let headers = config.validate_environment(&params, platform, &FixedSession);
    let mut output = Object::new();
    output.insert(String::from("headers"), json!(headers));
    output
}

fn params_value(config: &ChatGptConfig<Recording<Login>>, spec: &ParamsInput) -> Object {
    let catalog = CatalogCapabilities {
        chat_models: spec.chat_models.clone(),
        text_models: spec.text_models.clone(),
    };
    let mut mapped = Vec::new();
    for requested in &spec.requests {
        let mut optional = Object::new();
        config.map_openai_params(&spec.model, requested, &mut optional, &catalog);
        mapped.push(Value::Object(optional));
    }
    let mut output = Object::new();
    output.insert(String::from("params"), Value::Array(mapped));
    output
}

fn normalize_value(spec: &NormalizeInput) -> Result<Object, FixtureError> {
    let sync = ChatGptToolCallNormalizer::from(decode_chunks(&spec.chunks)?.into_iter())
        .collect::<Vec<_>>();
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let asynchronous = runtime.block_on(collect_async(decode_chunks(&spec.chunks)?));
    let mut output = Object::new();
    output.insert(String::from("sync"), project(&sync)?);
    output.insert(String::from("async"), project(&asynchronous)?);
    Ok(output)
}

fn step_value(
    input: &Input,
    config: &mut ChatGptConfig<Recording<Login>>,
) -> Result<Object, FixtureError> {
    Ok(match &input.step {
        Step::ProviderInfo(spec) => provider_value(config, spec)?,
        Step::Headers(spec) => headers_value(config, &input.platform, spec),
        Step::Params(spec) => params_value(config, spec),
        Step::Normalize(spec) => normalize_value(spec)?,
    })
}

fn run_step(input: &Input) -> Result<Value, FixtureError> {
    let (api_key, headers) = match &input.step {
        Step::Headers(spec) => (
            spec.api_key.clone().map(SecretString::from),
            spec.headers.clone(),
        ),
        Step::ProviderInfo(_) | Step::Params(_) | Step::Normalize(_) => (None, Headers::new()),
    };
    let mut config = assemble(input, api_key, headers)?;
    let mut output = step_value(input, &mut config)?;
    let calls = config
        .authenticator
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    output.insert(String::from("authentication_calls"), json!(calls));
    Ok(Value::Object(output))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run_step(input).map_err(|error| {
            Box::new(HelperError {
                class: String::from("ChatGptChatFixture"),
                status: None,
                error_type: Some(None),
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
