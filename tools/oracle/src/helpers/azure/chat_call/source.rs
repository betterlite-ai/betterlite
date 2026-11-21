use std::future::pending;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_azure::azure_stream_errors::records::{GetStreamClient, StreamClient};
use betterlite_azure::chat_call::client::{ClientHandle, ClientSource, GatewayClient, GetClient};
use betterlite_azure::endpoints::initialize::EndpointError;
use betterlite_azure::endpoints::projection::ClientKind;
use betterlite_openai::output_limits::raw_send::{
    AsyncRawChatTransport, RawChatError, RawChatResponse, SyncRawChatTransport,
};
use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};

pub(super) mod answer;

use answer::{Answer, ErrorKind};

type RawReply = Result<RawChatResponse, RawChatError>;
type Handled = Result<ClientHandle<ScriptedTransport>, EndpointError>;

pub(super) const CHAT_BODY: &str = concat!(
    r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"#,
    r#""model":"gpt-4o","choices":[{"index":0,"#,
    r#""message":{"role":"assistant","content":"Hello!"},"finish_reason":"stop"}],"#,
    r#""usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#
);

#[derive(Clone)]
pub(super) struct RequestRecord {
    pub data: Value,
    pub caller_headers: Json,
}

#[derive(Clone)]
pub(super) struct GetRecord {
    pub api_version: Option<String>,
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub is_async: bool,
    pub azure_ad_token: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout: Option<f64>,
    pub supplied: bool,
}

#[derive(Clone)]
pub(super) struct GatewayRecord {
    pub api_base: String,
    pub model: String,
    pub api_version: Option<String>,
    pub max_retries: Value,
    pub timeout: f64,
    pub api_key: Option<String>,
    pub acompletion: bool,
}

#[derive(Clone)]
pub(super) struct ScriptedTransport {
    answer: Answer,
    requests: Arc<Mutex<Vec<RequestRecord>>>,
}

impl ScriptedTransport {
    fn record(&self, data: &Map<String, Value>) {
        let mut body = data.clone();
        let caller = body.remove("extra_headers").map_or(Json::Null, |value| {
            serde_json::from_value(value).unwrap_or(Json::Null)
        });
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RequestRecord {
                data: Value::Object(body),
                caller_headers: caller,
            });
    }
}

impl SyncRawChatTransport for ScriptedTransport {
    fn create(&self, data: &Map<String, Value>, _timeout: f64) -> RawReply {
        self.record(data);
        self.answer.make()
    }
}

#[async_trait]
impl AsyncRawChatTransport for ScriptedTransport {
    async fn create(&self, data: &Map<String, Value>, timeout: f64) -> RawReply {
        if self.answer.error == Some(ErrorKind::Pending) {
            self.record(data);
            return pending::<RawReply>().await;
        }
        SyncRawChatTransport::create(self, data, timeout)
    }
}

pub(super) struct ScriptedSource {
    kind: ClientKind,
    api_key: Option<SecretString>,
    answer: Answer,
    requests: Arc<Mutex<Vec<RequestRecord>>>,
    gets: Arc<Mutex<Vec<GetRecord>>>,
    gateways: Arc<Mutex<Vec<GatewayRecord>>>,
}

impl ScriptedSource {
    pub(super) fn new(kind: ClientKind, api_key: Option<SecretString>, answer: Answer) -> Self {
        Self {
            kind,
            api_key,
            answer,
            requests: Arc::new(Mutex::new(Vec::new())),
            gets: Arc::new(Mutex::new(Vec::new())),
            gateways: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn handle(&self) -> ClientHandle<ScriptedTransport> {
        ClientHandle {
            kind: self.kind,
            api_key: self.api_key.clone(),
            base_url: "https://unit.test/openai/".to_owned(),
            transport: ScriptedTransport {
                answer: self.answer.clone(),
                requests: Arc::clone(&self.requests),
            },
        }
    }

    fn record<Record>(
        &self,
        slot: &Arc<Mutex<Vec<Record>>>,
        value: Record,
    ) -> ClientHandle<ScriptedTransport> {
        slot.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
        self.handle()
    }

    pub(super) fn requests(&self) -> Vec<RequestRecord> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn gets(&self) -> Vec<GetRecord> {
        self.gets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn gateways(&self) -> Vec<GatewayRecord> {
        self.gateways
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for ScriptedSource {
    type Client = ScriptedTransport;

    fn get(&self, call: GetClient<'_, ScriptedTransport>) -> Handled {
        Ok(self.record(
            &self.gets,
            GetRecord {
                api_version: call.api_version.map(str::to_owned),
                api_base: call.api_base.map(str::to_owned),
                api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
                model: call.model.map(str::to_owned),
                is_async: call.is_async,
                azure_ad_token: call
                    .litellm_params
                    .azure_ad_token
                    .as_ref()
                    .map(|token| token.expose_secret().to_owned()),
                max_retries: call.litellm_params.max_retries,
                timeout: call.litellm_params.timeout,
                supplied: call.supplied.is_some(),
            },
        ))
    }

    fn gateway(&self, call: GatewayClient<'_>) -> Handled {
        let api_base = call.api_base.to_owned();
        let api_version = call.api_version.map(str::to_owned);
        let max_retries = serde_json::to_value(&call.max_retries.0).unwrap_or(Value::Null);
        let api_key = call.api_key.map(|key| key.expose_secret().to_owned());
        Ok(self.record(
            &self.gateways,
            GatewayRecord {
                api_base,
                model: call.model.to_owned(),
                api_version,
                max_retries,
                timeout: call.timeout,
                api_key,
                acompletion: call.acompletion,
            },
        ))
    }

    fn stream(
        &self,
        _supplied: Option<&ClientHandle<Self::Client>>,
        _call: GetStreamClient<'_>,
    ) -> Result<StreamClient, EndpointError> {
        Err(EndpointError::MissingApiVersion)
    }
}

pub(super) fn kind_of(name: Option<&str>, asynchronous: bool) -> ClientKind {
    match name {
        Some("openai_sync") => ClientKind::OpenAi,
        Some("openai_async") => ClientKind::AsyncOpenAi,
        Some("other") => ClientKind::Other,
        Some("sync") => ClientKind::AzureOpenAi,
        Some("async") => ClientKind::AsyncAzureOpenAi,
        Some(_) | None => {
            if asynchronous {
                ClientKind::AsyncAzureOpenAi
            } else {
                ClientKind::AzureOpenAi
            }
        }
    }
}
