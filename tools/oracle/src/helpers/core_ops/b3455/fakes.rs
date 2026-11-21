use std::collections::VecDeque;
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
use thiserror::Error;

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
    ) -> Result<(SecretString, String), PortError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "project": project,
                "credentials": credentials.is_some(),
            }));
        Ok((
            SecretString::from(TOKEN.to_owned()),
            project.map(str::to_owned).unwrap_or_default(),
        ))
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

enum Part<'request> {
    Json(&'request JsonRequest<'request>),
    Get(&'request GetRequest<'request>),
    Multipart(&'request MultipartRequest<'request>),
}

impl Part<'_> {
    fn into_value(self) -> Value {
        match self {
            Part::Json(request) => json!({
                "kind": "post_json",
                "url": request.url,
                "timeout": request.timeout.as_secs_f64(),
                "json": serde_json::from_slice::<Value>(request.body).unwrap_or(Value::Null),
                "metadata": null,
                "filename": null,
                "file_bytes": null,
                "file_content_type": null,
                "headers": ["Authorization", "Content-Type"],
            }),
            Part::Get(request) => json!({
                "kind": "get",
                "url": request.url,
                "timeout": request.timeout.as_secs_f64(),
                "json": null,
                "metadata": null,
                "filename": null,
                "file_bytes": null,
                "file_content_type": null,
                "headers": ["Authorization"],
            }),
            Part::Multipart(request) => json!({
                "kind": "post_multipart",
                "url": request.url,
                "timeout": request.timeout.as_secs_f64(),
                "json": null,
                "metadata": serde_json::from_slice::<Value>(request.metadata).unwrap_or(Value::Null),
                "filename": request.filename,
                "file_bytes": request.file.len(),
                "file_content_type": request.file_content_type,
                "headers": ["Authorization", "X-Goog-Upload-Protocol"],
            }),
        }
    }
}

#[async_trait]
impl VertexRagHttp for ScriptedHttp {
    async fn post_json(&self, request: JsonRequest<'_>) -> Result<HttpReply, PortError> {
        self.push(Part::Json(&request).into_value());
        self.reply()
    }

    async fn get(&self, request: GetRequest<'_>) -> Result<HttpReply, PortError> {
        self.push(Part::Get(&request).into_value());
        self.reply()
    }

    async fn post_multipart(&self, request: MultipartRequest<'_>) -> Result<HttpReply, PortError> {
        self.push(Part::Multipart(&request).into_value());
        self.reply()
    }
}

#[derive(Debug, Error)]
#[error("the reply script is exhausted")]
struct ExhaustedScript;
