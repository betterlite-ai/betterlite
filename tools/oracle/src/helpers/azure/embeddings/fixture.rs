use std::collections::{BTreeMap, HashMap};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::embeddings::client::{
    ClientFactory, EmbeddingClient, Fields, RawEmbedding, SdkError,
};
use betterlite_azure::embeddings::logging::{
    EmbeddingLogger, PostCallRecord, PreCallRecord, ResponseKind,
};
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{ClientIdentity, ClientKind, ClientOutcome};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_logging::logging_pre_post::error::PrePostError;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) type Calls = Arc<Mutex<Vec<Value>>>;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Answer {
    Reply(Reply),
    Connection(BTreeMap<String, String>),
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(super) enum ClientName {
    #[serde(rename = "azure_open_ai")]
    Azure,
    #[serde(rename = "async_azure_open_ai")]
    AsyncAzure,
    #[serde(rename = "open_ai")]
    Plain,
    #[serde(rename = "async_open_ai")]
    AsyncPlain,
}

impl ClientName {
    pub(super) const fn kind(self) -> ClientKind {
        match self {
            Self::Azure => ClientKind::AzureOpenAi,
            Self::AsyncAzure => ClientKind::AsyncAzureOpenAi,
            Self::Plain => ClientKind::OpenAi,
            Self::AsyncPlain => ClientKind::AsyncOpenAi,
        }
    }
}

pub(super) struct Sdk {
    pub(super) kind: ClientKind,
    pub(super) answer: Answer,
    pub(super) calls: Calls,
}

// the SDK sends extra_headers as HTTP headers, so the wire body leaves them out
fn wire_view(data: &Fields) -> Value {
    let mut body = serde_json::to_value(data).unwrap_or(Value::Null);
    if let Some(fields) = body.as_object_mut() {
        fields.remove("extra_headers");
    }
    json!({"body": body})
}

// a case lists only the headers its provider sent; the Python fixture's httpx response adds the body's length
fn reply_headers(reply: &Reply) -> BTreeMap<String, String> {
    let mut headers: BTreeMap<String, String> = reply
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();
    headers
        .entry("content-length".to_owned())
        .or_insert_with(|| reply.body.len().to_string());
    headers
}

impl EmbeddingClient for Sdk {
    fn kind(&self) -> ClientKind {
        self.kind
    }

    fn create(
        &self,
        data: &Fields,
        _timeout: f64,
    ) -> impl Future<Output = Result<RawEmbedding, SdkError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(wire_view(data));
        ready(match &self.answer {
            Answer::Connection(_) => Err(SdkError {
                status_code: None,
                headers: None,
                response: None,
                message: "Connection error.".to_owned(),
            }),
            Answer::Reply(reply) => {
                let headers = reply_headers(reply);
                if reply.status >= 400 {
                    Err(SdkError::status(reply.status, headers, reply.body.clone()))
                } else {
                    Ok(RawEmbedding {
                        status: reply.status,
                        headers,
                        body: reply.body.clone().into_bytes(),
                    })
                }
            }
        })
    }
}

// every case supplies its client, so the factory is never asked to build one
pub(super) struct NoFactory;

impl ClientFactory for NoFactory {
    type Client = Sdk;

    fn client(&self, _outcome: &ClientOutcome<'_>) -> Result<Sdk, SdkError> {
        Err(SdkError {
            status_code: None,
            headers: None,
            response: None,
            message: "the helper supplies every client".to_owned(),
        })
    }
}

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) pre: Vec<Value>,
    pub(super) post: Vec<Value>,
}

// the oracle compares the response's model, object and data; the SDK object has no completion_tokens in its dump
fn original_view(record: &PostCallRecord) -> Value {
    if record.kind == ResponseKind::ErrorText {
        return Value::Null;
    }
    let dumped = serde_json::to_value(&record.original_response).unwrap_or(Value::Null);
    json!({
        "model": dumped.get("model"),
        "object": dumped.get("object"),
        "data": dumped.get("data"),
    })
}

impl EmbeddingLogger for Recorder {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(json!({
            "input": record.input,
            "api_key": record.api_key.as_ref().map(ExposeSecret::expose_secret),
            "azure_ad_token": record.azure_ad_token.as_ref().map(ExposeSecret::expose_secret),
            "complete_input_dict": record.complete_input_dict,
        }));
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(json!({
            "input": record.input,
            "api_key": record.api_key.as_ref().map(ExposeSecret::expose_secret),
            "kind": record.kind,
            "original_response": original_view(&record),
            "complete_input_dict": record.complete_input_dict,
            "api_base": record.api_base,
        }));
    }
}

#[derive(Default)]
pub(super) struct Cache(HashMap<ClientIdentity, ClientKind>);

impl ClientCache for Cache {
    fn get(&self, identity: &ClientIdentity) -> Option<ClientKind> {
        self.0.get(identity).copied()
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, _owned: bool) {
        self.0.insert(identity, kind);
    }
}

fn no_source() -> SourceFailure {
    SourceFailure {
        class: "RuntimeError".to_owned(),
        status: None,
        message: "the helper has no credential source".to_owned(),
    }
}

pub(super) struct NoSources;

type Provider<'source> = Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

impl CredentialSources for NoSources {
    fn client_secret(&self, _record: &ClientSecretRecord) -> Provider<'_> {
        Err(no_source())
    }

    fn username_password(&self, _record: &UsernamePasswordRecord) -> Provider<'_> {
        Err(no_source())
    }

    fn sdk_refresh(&self, _scope: &str) -> Provider<'_> {
        Err(no_source())
    }

    fn default_credential(&self, _scope: &str) -> Option<Arc<dyn CallableTokenProvider + '_>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(no_source())
    }
}
