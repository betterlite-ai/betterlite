mod client;
mod server;

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_ocr::azure_doc_intel::AzureDocIntelligenceOcrConfig;
use betterlite_ocr::json::{Json, Object};
use betterlite_ocr::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use betterlite_ocr::ocr_config::{OCR_RESPONSE_MAX_BYTES, PreparedOcrRequest, Secrets};
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::ocr_http::{CallHooks, OcrWireRequest, ocr};
use betterlite_ocr::ocr_settings::OcrSettings;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use server::{Captured, Reply, Server};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    document: OcrDocument,
    #[serde(default)]
    options: Object,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    analyze: ReplyInput,
    #[serde(default)]
    polls: Vec<ReplyInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyInput {
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<Json>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(&error))?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> HelperResult {
    let server = Server::start(replies(input)?).await?;
    let request = prepared_request(input, &server.base);
    let response = ocr(
        &AzureDocIntelligenceOcrConfig,
        &client::ocr_client()?,
        &request,
        &NoHooks,
    )
    .await
    .map_err(|error| ocr_failure(&error))?;
    let seen = server.finish();
    Ok(json!({
        "requests": requests_json(&seen)?,
        "response": serde_json::to_value(&response).map_err(|error| failure(&error))?,
        "native": serde_json::to_value(&response.provider_native_response)
            .map_err(|error| failure(&error))?,
    }))
}

fn replies(input: &Input) -> Result<Vec<Reply>, Box<HelperError>> {
    let mut replies = vec![reply(&input.analyze)?];
    for spec in &input.polls {
        replies.push(reply(spec)?);
    }
    Ok(replies)
}

fn reply(spec: &ReplyInput) -> Result<Reply, Box<HelperError>> {
    let body = match &spec.body {
        Some(body) => serde_json::to_vec(body).map_err(|error| failure(&error))?,
        None => Vec::new(),
    };
    Ok(Reply {
        status: spec.status,
        headers: spec
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        body,
    })
}

fn prepared_request(input: &Input, base: &str) -> PreparedOcrRequest {
    let credentials = ResolvedOcrCredentials {
        api_key: input.api_key.as_ref().map(|key| OcrSourced {
            value: SecretString::from(key.as_str()),
            source: OcrInputSource::Request,
        }),
        api_base: Some(OcrSourced {
            value: base.to_owned(),
            source: OcrInputSource::Deployment,
        }),
    };
    let mut secrets = Secrets::new();
    for (name, secret) in &input.environment {
        let _previous = secrets.insert(name.clone(), SecretString::from(secret.as_str()));
    }
    let transport = OcrTransportConfig {
        extra_headers: input
            .extra_headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        extra_headers_source: OcrInputSource::Request,
        timeout: Some(Duration::from_secs(15)),
        max_response_bytes: OCR_RESPONSE_MAX_BYTES,
    };
    PreparedOcrRequest {
        model: input.model.clone(),
        document: input.document.clone(),
        connection: OcrConnection::new(credentials, transport, OcrSettings::default(), secrets),
        caller_document: false,
        optional_params: input.options.clone(),
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    }
}

fn requests_json(seen: &[Captured]) -> Result<Value, Box<HelperError>> {
    let mut listed = Vec::with_capacity(seen.len());
    for request in seen {
        let body = if request.body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&request.body).map_err(|error| failure(&error))?
        };
        listed.push(json!({
            "method": request.method,
            "path": request.path,
            "authorization": request.headers.get("authorization"),
            "subscription_key": request.headers.get("ocp-apim-subscription-key"),
            "content_type": request.headers.get("content-type"),
            "body": body,
        }));
    }
    Ok(Value::Array(listed))
}

fn ocr_failure(error: &OcrError) -> Box<HelperError> {
    let fields = error.fields();
    Box::new(HelperError {
        class: "OcrError".to_owned(),
        status: Some(fields.status),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

pub(super) fn failure(error: &dyn std::fmt::Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: "B6805Oracle".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

struct NoHooks;

#[async_trait]
impl CallHooks for NoHooks {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError> {
        Ok(request)
    }

    async fn response_received(&self, _body: &[u8]) -> Result<(), OcrError> {
        Ok(())
    }
}
