use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::count_tokens::counter::{TokenCountResponse, TokenCounter as _};
use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_anthropic::count_tokens::wire::CountTokensBody;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::anthropic_count_tokens::Raw as BetaRaw;
use betterlite_config::sections::azure_credentials::{DeploymentSettings, Raw as CredentialRaw};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::anthropic_count::counter::{AzureAnthropicTokenCounter, CountTokensCall};
use crate::anthropic_count::error::ApiError;
use crate::anthropic_count::handler::{CountTokensRequest, FoundryCountHandler};
use crate::anthropic_messages::environment::{ParamsKey, TokenSource};

pub(super) const MODEL: &str = "claude-sonnet-4-5";
const API_BASE: &str = "https://r.services.ai.azure.com";
const USER_MESSAGE: &str = r#"[{"role":"user","content":"hi"}]"#;
pub(super) const ARGUMENT_KEY: &str = "argument-key";
pub(super) const PARAMS_KEY: &str = "params-key";
pub(super) const TOKENIZER_TYPE: &str = "azure_ai_anthropic_api";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Json,
    pub(super) timeout: Duration,
}

pub(super) struct Script {
    pub(super) reply: Option<(u16, String)>,
    pub(super) seen: Arc<Mutex<Vec<Seen>>>,
}

impl Debug for Script {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Script").finish_non_exhaustive()
    }
}

impl Script {
    fn record(&self, request: OutboundRequest) {
        let OutboundRequest {
            url,
            headers,
            body,
            deadline,
            ..
        } = request;
        let payload = match body {
            RequestBody::Json(bytes) => {
                serde_json::from_slice(&bytes).expect("the request body is JSON")
            }
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Json::Null
            }
        };
        let pairs = headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                url: url.to_string(),
                headers: pairs,
                body: payload,
                timeout: deadline,
            });
    }
}

impl CountTokensTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        ready(
            self.reply
                .clone()
                .map_or(Err(TransportError::Closed), |(status, text)| {
                    Ok(UpstreamResponse {
                        status,
                        headers: HeaderMap::new(),
                        body: text.into_bytes(),
                    })
                }),
        )
    }
}

// The litellm_params api_key entry as LiteLLM received it: missing, null, or a value
pub(super) enum Params {
    Absent,
    Null,
    Key(SecretString),
    Typed(Option<SecretString>),
}

pub(super) struct Call {
    pub(super) api_key: SecretString,
    pub(super) api_base: Option<String>,
    pub(super) params: Params,
    pub(super) configured_key: Option<SecretString>,
    pub(super) token: Option<String>,
    pub(super) environment_key: Option<SecretString>,
    pub(super) environment_base: Option<String>,
    pub(super) messages: Option<Json>,
    pub(super) system: Option<Json>,
    pub(super) tools: Option<Json>,
    pub(super) model: String,
    pub(super) request_model: String,
    pub(super) timeout: Option<Duration>,
    pub(super) request_timeout: Duration,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) reply: Option<(u16, String)>,
}

impl Call {
    pub(super) fn standard() -> Self {
        Self {
            api_key: SecretString::from(ARGUMENT_KEY),
            api_base: Some(API_BASE.to_owned()),
            params: Params::Key(SecretString::from(PARAMS_KEY)),
            configured_key: None,
            token: None,
            environment_key: None,
            environment_base: None,
            messages: Some(serde_json::from_str::<Json>(USER_MESSAGE).expect("valid fixture JSON")),
            system: None,
            tools: None,
            model: MODEL.to_owned(),
            request_model: MODEL.to_owned(),
            timeout: None,
            request_timeout: Duration::from_mins(100),
            environment: BTreeMap::new(),
            reply: Some((200, r#"{"input_tokens":5}"#.to_owned())),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CounterOutcome {
    pub(super) selected: [bool; 3],
    pub(super) result: Option<TokenCountResponse>,
    pub(super) seen: Option<Seen>,
    pub(super) warnings: Vec<Warning>,
}

#[derive(Debug)]
pub(super) struct HandleOutcome {
    pub(super) result: Result<Json, ApiError>,
    pub(super) seen: Option<Seen>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Warning {
    pub(super) subject: String,
    pub(super) status: Option<u16>,
}

impl From<&Call> for FoundryCountHandler<Script> {
    fn from(call: &Call) -> Self {
        Self {
            transport: Script {
                reply: call.reply.clone(),
                seen: Arc::new(Mutex::new(Vec::new())),
            },
            request_timeout: call.request_timeout,
            beta_version: beta_version(&call.environment),
            configured_key: call.configured_key.clone(),
        }
    }
}

pub(super) fn run_counter(call: &Call) -> CounterOutcome {
    let counter = AzureAnthropicTokenCounter {
        handler: FoundryCountHandler::from(call),
        environment_api_key: call.environment_key.clone(),
        environment_api_base: call.environment_base.clone(),
    };
    let (result, warnings) = with_token(call, |token| {
        let data = CountTokensCall {
            model_to_use: &call.model,
            messages: call.messages.as_ref(),
            contents: None,
            params_key: params_key(&call.params),
            api_base: call.api_base.as_deref(),
            token,
            request_model: &call.request_model,
            tools: call.tools.as_ref(),
            system: call.system.as_ref(),
        };
        captured(|| run_blocking(counter.count_tokens(&data)))
    });
    CounterOutcome {
        selected: [
            counter.should_use_token_counting_api(Some("azure_ai")),
            counter.should_use_token_counting_api(Some("anthropic")),
            counter.should_use_token_counting_api(None),
        ],
        result,
        seen: first_request(&counter.handler.transport),
        warnings,
    }
}

pub(super) fn run_handle(call: &Call) -> HandleOutcome {
    let messages = call.messages.clone().unwrap_or(Json::Null);
    let handler = FoundryCountHandler::from(call);
    let outcome = with_token(call, |token| {
        let request = CountTokensRequest {
            body: CountTokensBody {
                model: &call.model,
                messages: &messages,
                tools: call.tools.as_ref(),
                system: call.system.as_ref(),
                optional_params: &IndexMap::new(),
            },
            api_key: &call.api_key,
            api_base: call.api_base.as_deref().unwrap_or_default(),
            params_key: params_key(&call.params),
            token,
            timeout: call.timeout,
        };
        run_blocking(handler.handle_count_tokens_request(&request))
    });
    HandleOutcome {
        result: outcome.map(|reply| reply.document),
        seen: first_request(&handler.transport),
    }
}

fn with_token<R, B>(call: &Call, body: B) -> R
where
    B: FnOnce(&TokenSource<'_>) -> R,
{
    let credentials = credential_settings(call.token.as_deref());
    let token = TokenSource {
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &Offline,
    };
    body(&token)
}

fn run_blocking<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .expect("the test runtime")
        .block_on(future)
}

fn first_request(transport: &Script) -> Option<Seen> {
    transport
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned()
}

const fn params_key(params: &Params) -> ParamsKey<'_> {
    match params {
        Params::Absent => ParamsKey::DictAbsent,
        Params::Null => ParamsKey::DictNull,
        Params::Key(secret) => ParamsKey::Dict(secret),
        Params::Typed(None) => ParamsKey::Typed(None),
        Params::Typed(Some(secret)) => ParamsKey::Typed(Some(secret)),
    }
}

pub(super) fn handle_headers(call: &Call) -> BTreeMap<String, String> {
    let outcome = run_handle(call);
    let seen = outcome
        .seen
        .expect("the fixture transport recorded a request");
    seen.headers
}

pub(super) fn beta_version(environment: &BTreeMap<String, String>) -> String {
    let raw = BetaRaw::default();
    let settings = raw.convert(environment).expect("the beta settings");
    settings.beta_version
}

fn credential_settings(azure_ad_token: Option<&str>) -> DeploymentSettings {
    let token = azure_ad_token.map_or_else(|| "null".to_owned(), |token| format!("{token:?}"));
    let source = format!(r#"{{"model_list":[{{"litellm_params":{{"azure_ad_token":{token}}}}}]}}"#);
    let raw: CredentialRaw = serde_json::from_str(&source).expect("the credential fixture");
    let settings = raw
        .convert(&BTreeMap::new())
        .expect("the credential settings");
    settings
        .deployments
        .into_iter()
        .next()
        .expect("one deployment")
}

// The count path never reaches Azure, so every token source refuses
struct Offline;

fn refuse() -> SourceFailure {
    SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: "the count path must not reach a token source".to_owned(),
    }
}

impl CredentialSources for Offline {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(refuse())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(refuse())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(refuse())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(refuse())
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    fn entries(&self) -> Vec<Warning> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|message| warning(message))
            .collect()
    }
}

fn warning(message: &str) -> Warning {
    let (subject, detail) = message.split_once(": ").unwrap_or((message, ""));
    let status = detail
        .strip_prefix("status=")
        .and_then(|rest| rest.split(',').next())
        .and_then(|number| number.parse::<u16>().ok());
    Warning {
        subject: subject.to_owned(),
        status,
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn captured<R, B>(body: B) -> (R, Vec<Warning>)
where
    B: FnOnce() -> R,
{
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let outcome = with_default(subscriber, body);
    let entries = logs.entries();
    (outcome, entries)
}
