use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::completion::client::{
    ClientHandle, ClientSource, CompletionsTransport, GatewayClient, GetClient,
};
use betterlite_azure::endpoints::initialize::EndpointError;
use betterlite_azure::endpoints::projection::ClientKind;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::sdk::{RawCompletion, SdkError, StatusError};
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde::Deserialize;

const BODY: &str = concat!(
    r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"#,
    r#""model":"gpt-35-turbo-instruct","#,
    r#""choices":[{"text":"Hello!","index":0,"logprobs":null,"finish_reason":"stop"}],"#,
    r#""usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#
);

// The answer the fixture transport hands the handler: a buffered body or the SSE events of a stream
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Answer {
    status: Option<u16>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    body: Option<String>,
    events: Option<Vec<String>>,
}

// The client the SDK constructs for the case: its kind, key and base url, plus the answer it returns
#[derive(Clone)]
pub(crate) struct ScriptedSource {
    kind: ClientKind,
    api_key: Option<SecretString>,
    base_url: String,
    answer: Answer,
    creates: Arc<Mutex<Vec<Json>>>,
}

impl ScriptedSource {
    pub(super) fn new(
        kind: ClientKind,
        api_key: Option<SecretString>,
        base_url: &str,
        answer: Answer,
    ) -> Self {
        Self {
            kind,
            api_key,
            base_url: base_url.to_owned(),
            answer,
            creates: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn handle(&self) -> ClientHandle<ScriptedTransport> {
        ClientHandle {
            kind: self.kind,
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            transport: ScriptedTransport {
                answer: self.answer.clone(),
                creates: Arc::clone(&self.creates),
            },
        }
    }

    pub(super) fn requests(&self) -> Vec<Json> {
        self.creates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for ScriptedSource {
    type Client = ScriptedTransport;

    fn get(&self, _call: GetClient<'_>) -> Result<ClientHandle<Self::Client>, EndpointError> {
        Ok(self.handle())
    }

    fn gateway(
        &self,
        _call: GatewayClient<'_>,
    ) -> Result<ClientHandle<Self::Client>, EndpointError> {
        Ok(self.handle())
    }
}

#[derive(Clone)]
pub(super) struct ScriptedTransport {
    answer: Answer,
    creates: Arc<Mutex<Vec<Json>>>,
}

impl CompletionsTransport for ScriptedTransport {
    fn create<'call>(
        &'call self,
        data: &'call Extra,
        _timeout: Option<f64>,
    ) -> Pin<Box<dyn Future<Output = Result<RawCompletion, SdkError>> + Send + 'call>> {
        let mut record = Extra::new();
        record.insert("data".to_owned(), Json::Object(data.clone()));
        self.creates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Json::Object(record));
        let answer = answer_once(&self.answer);
        Box::pin(async move { answer })
    }
}

fn header_map(headers: &BTreeMap<String, String>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (raw_name, raw_value) in headers {
        if let (Ok(name), Ok(value)) = (
            raw_name.parse::<HeaderName>(),
            raw_value.parse::<HeaderValue>(),
        ) {
            map.insert(name, value);
        }
    }
    map
}

// the message the OpenAI SDK builds for a non-JSON error body, so the handler passes it through unchanged
fn sdk_message(status: u16, body: &str) -> String {
    let text = body.trim();
    if text.is_empty() {
        format!("Error code: {status}")
    } else {
        text.to_owned()
    }
}

fn answer_once(answer: &Answer) -> Result<RawCompletion, SdkError> {
    let status = answer.status.unwrap_or(200);
    let headers = header_map(&answer.headers);
    if status >= 400 {
        let body = answer.body.clone().unwrap_or_default();
        return Err(SdkError::Status(Box::new(StatusError {
            status: Some(status),
            message: sdk_message(status, &body),
            headers,
            error_type: None,
            param: None,
            code: None,
        })));
    }
    if let Some(events) = &answer.events {
        let items: Vec<Result<Vec<u8>, SdkError>> = events
            .iter()
            .map(|event| Ok(event.clone().into_bytes()))
            .collect();
        return Ok(RawCompletion::Stream {
            status,
            headers,
            body: Box::pin(stream::iter(items)),
        });
    }
    let body = answer.body.clone().unwrap_or_else(|| BODY.to_owned());
    Ok(RawCompletion::Buffered {
        status,
        headers,
        body: body.into_bytes(),
    })
}
