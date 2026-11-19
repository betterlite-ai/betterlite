use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_client_secret::SecretResolver;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::azure_stream_errors::error::{Raised, Reported};
use betterlite_azure::azure_stream_errors::records::{
    ChatStreamTransport, GetStreamClient, StreamAnswer, StreamClient, StreamSource,
};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use betterlite_azure::endpoints::projection::{CallableProvider, ClientKind};
use betterlite_azure::oidc_exchange::transport::{Clock, HttpReply, Transport};
use betterlite_azure::oidc_exchange::{AzureOidcExchange, OidcExchangeInput};
use betterlite_config::sections::azure_oidc_exchange::Settings;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::sdk::{ByteStream, SdkError};
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::{ClientKindSpec, ErrorSpec, ExchangeSpec, SendSpec, StreamCase};

pub(super) struct GetView {
    pub(super) api_version: String,
    pub(super) api_base: String,
    pub(super) api_key: Option<String>,
    pub(super) model: String,
    pub(super) is_async: bool,
    pub(super) litellm_params: Json,
}

pub(super) struct FormView {
    pub(super) url: String,
    pub(super) form: BTreeMap<String, String>,
}

fn kind_of(spec: &ClientKindSpec) -> ClientKind {
    match spec {
        ClientKindSpec::AzureSync => ClientKind::AzureOpenAi,
        ClientKindSpec::AzureAsync => ClientKind::AsyncAzureOpenAi,
        ClientKindSpec::OpenAiSync => ClientKind::OpenAi,
        ClientKindSpec::OpenAiAsync => ClientKind::AsyncOpenAi,
    }
}

pub(super) fn raised_of(spec: &ErrorSpec) -> Raised {
    let status = if spec.status.present {
        Reported::Present(spec.status.value)
    } else {
        Reported::Absent
    };
    let message = if spec.message.present {
        Reported::Present(spec.message.value.clone())
    } else {
        Reported::Absent
    };
    let mut raised = Raised::raised(
        "ScriptedError".to_owned(),
        status,
        message,
        spec.text.clone(),
    );
    raised.headers = spec
        .headers
        .value
        .as_ref()
        .map(|fields| SecretHeaders(fields.clone()));
    raised.body = if spec.body.present {
        Some(spec.body.value.clone().unwrap_or(Json::Null))
    } else {
        None
    };
    raised.response_headers = spec
        .response
        .value
        .as_ref()
        .map(|fields| SecretHeaders(fields.clone()));
    raised
}

fn send_headers(spec: &SendSpec) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in &spec.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).expect("send header name");
        let value = HeaderValue::from_str(value).expect("send header value");
        map.insert(name, value);
    }
    map
}

pub(super) struct ScriptedTransport {
    headers: HeaderMap,
    payload: Vec<u8>,
    requests: Arc<Mutex<Vec<Extra>>>,
}

impl ChatStreamTransport for ScriptedTransport {
    fn create<'call>(
        &'call self,
        data: &'call Extra,
        _timeout: Option<f64>,
    ) -> Pin<Box<dyn Future<Output = Result<StreamAnswer, Raised>> + Send + 'call>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data.clone());
        let headers = self.headers.clone();
        let payload = self.payload.clone();
        Box::pin(async move {
            let items: Vec<Result<Vec<u8>, SdkError>> = vec![Ok(payload)];
            let body: ByteStream = Box::pin(stream::iter(items));
            Ok(StreamAnswer { headers, body })
        })
    }
}

pub(super) struct ScriptedSource {
    pub(super) client: StreamClient,
    get_error: Option<Raised>,
    pub(super) gets: Arc<Mutex<Vec<GetView>>>,
    pub(super) requests: Arc<Mutex<Vec<Extra>>>,
}

// one entry per HTTP request: the body the transport was handed
pub(super) fn requests(source: &ScriptedSource) -> Vec<Json> {
    source
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|data| {
            let mut fields = Extra::new();
            fields.insert("data".to_owned(), Json::Object(data.clone()));
            Json::Object(fields)
        })
        .collect()
}

impl ScriptedSource {
    pub(super) fn new(case: &StreamCase) -> Self {
        let spec = case.supplied.as_ref().expect("the case supplies a client");
        let key = spec.api_key.clone().map(SecretString::from);
        let kind = kind_of(&spec.kind);
        let events = case.send.events.concat().into_bytes();
        let payload = if case.send.events.is_empty() {
            case.send
                .body
                .as_deref()
                .unwrap_or_default()
                .as_bytes()
                .to_vec()
        } else {
            events
        };
        let requests = Arc::new(Mutex::new(Vec::new()));
        let transport = ScriptedTransport {
            headers: send_headers(&case.send),
            payload,
            requests: Arc::clone(&requests),
        };
        ScriptedSource {
            client: StreamClient {
                kind,
                api_key: key,
                transport: Arc::new(transport),
            },
            get_error: case.get_error.as_ref().map(raised_of),
            gets: Arc::new(Mutex::new(Vec::new())),
            requests,
        }
    }
}

fn litellm_params_view(params: &betterlite_azure::endpoints::projection::LiteLlmParams) -> Json {
    let mut fields = Extra::new();
    fields.insert(
        "azure_ad_token".to_owned(),
        params.azure_ad_token.as_ref().map_or(Json::Null, |token| {
            Json::Text(token.expose_secret().to_owned())
        }),
    );
    fields.insert(
        "max_retries".to_owned(),
        params.max_retries.map_or(Json::Null, |value| {
            Json::Number(serde_json::Number::from(value))
        }),
    );
    fields.insert(
        "timeout".to_owned(),
        match params.timeout {
            Some(value) => serde_json::Number::from_f64(value).map_or(Json::Null, Json::Number),
            None => Json::Null,
        },
    );
    Json::Object(fields)
}

impl StreamSource for ScriptedSource {
    fn get(&self, request: GetStreamClient<'_>) -> Result<StreamClient, Raised> {
        // get_azure_openai_client raises before recording anything, so a failure leaves no get
        if let Some(error) = &self.get_error {
            return Err(error.clone());
        }
        self.gets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(GetView {
                api_version: request.api_version.to_owned(),
                api_base: request.api_base.to_owned(),
                api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
                model: request.model.to_owned(),
                is_async: request.is_async,
                litellm_params: litellm_params_view(request.litellm_params),
            });
        Ok(self.client.clone())
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> f64 {
        1_700_000_000.0
    }
}

struct RecordingTransport {
    status: u16,
    body: String,
    forms: Arc<Mutex<Vec<FormView>>>,
}

impl Transport for RecordingTransport {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<HttpReply> {
        let fields: BTreeMap<String, String> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        self.forms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FormView {
                url: url.to_owned(),
                form: fields,
            });
        Ok(HttpReply {
            status: self.status,
            body: SecretString::from(self.body.clone()),
        })
    }
}

struct FixedResolver {
    assertion: SecretString,
}

impl SecretResolver for FixedResolver {
    fn get_secret_str(&self, _reference: &str) -> Option<SecretString> {
        if self.assertion.expose_secret().is_empty() {
            return None;
        }
        Some(self.assertion.clone())
    }
}

pub(super) struct ScriptedSources {
    exchange: AzureOidcExchange,
    resolver: FixedResolver,
    pub(super) forms: Arc<Mutex<Vec<FormView>>>,
}

impl ScriptedSources {
    pub(super) fn new(spec: &ExchangeSpec) -> Self {
        let forms = Arc::new(Mutex::new(Vec::new()));
        let exchange = AzureOidcExchange::new(
            Settings::default(),
            spec.environment.clone(),
            Arc::new(RecordingTransport {
                status: spec.status,
                body: spec.body.clone(),
                forms: Arc::clone(&forms),
            }),
            Arc::new(FixedClock),
        )
        .expect("the OIDC exchange builds");
        ScriptedSources {
            exchange,
            resolver: FixedResolver {
                assertion: SecretString::from(spec.assertion.clone()),
            },
            forms,
        }
    }
}

fn unscripted() -> SourceFailure {
    SourceFailure {
        class: "CredentialSources".to_owned(),
        status: None,
        message: "not scripted".to_owned(),
    }
}

impl CredentialSources for ScriptedSources {
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

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.exchange
            .token(input, &self.resolver)
            .map_err(|error| SourceFailure {
                class: "AzureOpenAIError".to_owned(),
                status: error.status_code(),
                message: error.to_string(),
            })
    }
}

struct DummyProvider;

impl CallableTokenProvider for DummyProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Err(ProviderFailure {
            class: "Dummy".to_owned(),
            message: "not scripted".to_owned(),
        })
    }
}

pub(super) fn callable_provider(identity: u64) -> CallableProvider {
    CallableProvider {
        provider: Arc::new(DummyProvider),
        identity,
    }
}
