use std::collections::VecDeque;
use std::error::Error as StdError;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::credentials::CredentialValues;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::rag::ingestion::credentials::{CredentialLookup, CredentialLookupError};
use betterlite_core::rag::ingestion::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use betterlite_core::rag::ingestion::upload::{FetchedResponse, UrlFetch, UrlFetchError};
use betterlite_core::rag::vertex_ingestion::ports::{
    GetRequest, HttpReply, JsonRequest, MultipartRequest, PortError, VertexRagHttp, VertexTokenPort,
};
use secrecy::SecretString;
use serde_json::{Value, json};

const TOKEN: &str = "oracle-token";

pub(crate) struct NoOcr;

#[async_trait]
impl OcrOperation for NoOcr {
    async fn aocr(&self, _request: OcrRequest) -> Result<OcrResponse, OcrError> {
        Err(OcrError::Failed {
            message: "the store never calls OCR".to_owned(),
        })
    }
}

pub(crate) struct NoFetch;

#[async_trait]
impl UrlFetch for NoFetch {
    async fn fetch(&self, _url: &str) -> Result<FetchedResponse, UrlFetchError> {
        Err(UrlFetchError::Failed {
            message: "the store never fetches a URL".to_owned(),
        })
    }
}

pub(crate) struct NoCredentials;

impl CredentialLookup for NoCredentials {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        Err(CredentialLookupError::NotFound {
            credential_name: credential_name.to_owned(),
        })
    }
}

#[derive(Default)]
pub(crate) struct ScriptedToken {
    calls: Mutex<Vec<Value>>,
}

impl ScriptedToken {
    pub(crate) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl VertexTokenPort for ScriptedToken {
    async fn access_token(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
    ) -> Result<SecretString, PortError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "project": project,
                "credentials": credentials.is_some(),
            }));
        Ok(SecretString::from(TOKEN.to_owned()))
    }
}

pub(crate) struct ScriptedHttp {
    replies: Mutex<VecDeque<(u16, String)>>,
    requests: Mutex<Vec<Value>>,
}

impl ScriptedHttp {
    pub(crate) fn new(replies: Vec<(u16, String)>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn push(&self, request: Value) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    fn reply(&self) -> Result<HttpReply, PortError> {
        let popped = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some((status, body)) = popped else {
            return Err(PortError::new(ExhaustedScript));
        };
        Ok(HttpReply {
            status,
            body: body.into_bytes(),
        })
    }
}

#[async_trait]
impl VertexRagHttp for ScriptedHttp {
    async fn post_json(&self, request: JsonRequest<'_>) -> Result<HttpReply, PortError> {
        self.push(record(vec![
            ("kind", Value::String("post_json".to_owned())),
            ("url", Value::String(request.url.to_owned())),
            ("timeout", json!(request.timeout.as_secs_f64())),
            ("json", json_body(request.body)),
            ("metadata", Value::Null),
            ("filename", Value::Null),
            ("file_bytes", Value::Null),
            ("file_content_type", Value::Null),
        ]));
        self.reply()
    }

    async fn get(&self, request: GetRequest<'_>) -> Result<HttpReply, PortError> {
        self.push(record(vec![
            ("kind", Value::String("get".to_owned())),
            ("url", Value::String(request.url.to_owned())),
            ("timeout", json!(request.timeout.as_secs_f64())),
            ("json", Value::Null),
            ("metadata", Value::Null),
            ("filename", Value::Null),
            ("file_bytes", Value::Null),
            ("file_content_type", Value::Null),
        ]));
        self.reply()
    }

    async fn post_multipart(&self, request: MultipartRequest<'_>) -> Result<HttpReply, PortError> {
        self.push(record(vec![
            ("kind", Value::String("post_multipart".to_owned())),
            ("url", Value::String(request.url.to_owned())),
            ("timeout", json!(request.timeout.as_secs_f64())),
            ("json", Value::Null),
            ("metadata", json_body(request.metadata)),
            ("filename", Value::String(request.filename.to_owned())),
            ("file_bytes", json!(request.file.len())),
            (
                "file_content_type",
                Value::String(request.file_content_type.to_owned()),
            ),
        ]));
        self.reply()
    }
}

fn record(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn json_body(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap_or(Value::Null)
}

#[derive(Debug)]
struct ExhaustedScript;

impl Display for ExhaustedScript {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str("the reply script is exhausted")
    }
}

impl StdError for ExhaustedScript {}
