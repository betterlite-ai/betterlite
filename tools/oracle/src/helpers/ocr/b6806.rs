mod client;
mod server;

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_ocr::json::Object;
use betterlite_ocr::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use betterlite_ocr::ocr_config::{PreparedOcrRequest, Secrets};
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::ocr_http::{CallHooks, OcrWireRequest, ocr};
use betterlite_ocr::ocr_settings::OcrSettings;
use betterlite_ocr::reducto_ocr::{ReductoParseLegacyConfig, ReductoParseV3Config};
use betterlite_ocr::response::OCRResponse;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::HelperError;
use crate::multipart_oracle::Part;
use crate::multipart_oracle::capture::decode_multipart_capture;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(&error))?;
        runtime.block_on(execute(input))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    document: OcrDocument,
    api_key: Option<String>,
    optional_params: Object,
    upload: Reply,
    parse: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: Value,
}

struct NoHooks;

#[async_trait::async_trait]
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

async fn execute(input: &Input) -> crate::helpers::HelperResult {
    let (provider, model) = input
        .model
        .split_once('/')
        .ok_or_else(|| failure("the model must carry its provider prefix"))?;
    if provider != "reducto" {
        return Err(failure("the oracle only drives the reducto provider"));
    }
    let server = server::Server::start(replies(input)?).await?;
    let request = prepared_request(input, model, &server.base);
    let client = client::ocr_client()?;
    let response = if model.eq_ignore_ascii_case("parse-legacy") {
        ocr(
            &ReductoParseLegacyConfig::default(),
            &client,
            &request,
            &NoHooks,
        )
        .await
    } else {
        ocr(
            &ReductoParseV3Config::default(),
            &client,
            &request,
            &NoHooks,
        )
        .await
    }
    .map_err(|error| ocr_failure(&error))?;
    let captured = server.finish().await?;
    Ok(json!({
        "requests": observed(&captured)?,
        "response": response_json(&response)?,
    }))
}

fn replies(input: &Input) -> Result<Vec<server::Reply>, Box<HelperError>> {
    let upload = reply(&input.upload)?;
    let parse = reply(&input.parse)?;
    let source = input.document.source();
    if source.starts_with("reducto://") {
        return Ok(vec![parse]);
    }
    if source.starts_with("data:") {
        return Ok(vec![upload, parse]);
    }
    Ok(Vec::new())
}

fn reply(reply: &Reply) -> Result<server::Reply, Box<HelperError>> {
    let body = serde_json::to_vec(&reply.body).map_err(|error| failure(&error))?;
    Ok(server::Reply {
        status: reply.status,
        body,
    })
}

fn prepared_request(input: &Input, model: &str, base: &str) -> PreparedOcrRequest {
    let api_key = input
        .api_key
        .as_ref()
        .map(|key| SecretString::from(key.as_str()));
    let credentials = ResolvedOcrCredentials {
        api_key: api_key.map(|value| OcrSourced {
            value,
            source: OcrInputSource::Request,
        }),
        api_base: Some(OcrSourced {
            value: base.to_owned(),
            source: OcrInputSource::Request,
        }),
    };
    PreparedOcrRequest {
        model: model.to_owned(),
        document: input.document.clone(),
        connection: OcrConnection::new(
            credentials,
            OcrTransportConfig::default(),
            OcrSettings::default(),
            Secrets::new(),
        ),
        caller_document: false,
        optional_params: input.optional_params.clone(),
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    }
}

fn observed(captured: &[server::Captured]) -> Result<Value, Box<HelperError>> {
    let mut requests = Vec::with_capacity(captured.len());
    for request in captured {
        requests.push(observed_request(request)?);
    }
    Ok(Value::Array(requests))
}

fn observed_request(request: &server::Captured) -> Result<Value, Box<HelperError>> {
    let content_type = request
        .headers
        .get("content-type")
        .map_or("", String::as_str);
    if !content_type.starts_with("multipart/") {
        let body: Value = serde_json::from_slice(&request.body).map_err(|error| failure(&error))?;
        return Ok(json!({
            "method": request.method,
            "path": request.path,
            "authorization": request.headers.get("authorization").map(String::as_str),
            "content_type": content_type,
            "parts": Value::Null,
            "body": body,
        }));
    }
    let parts =
        decode_multipart_capture(content_type, &request.body).map_err(|error| failure(&error))?;
    let listed: Vec<Value> = parts.iter().map(part_json).collect();
    Ok(json!({
        "method": request.method,
        "path": request.path,
        "authorization": request.headers.get("authorization").map(String::as_str),
        "content_type": "multipart/form-data",
        "parts": listed,
        "body": Value::Null,
    }))
}

fn part_json(part: &Part) -> Value {
    json!({
        "name": part.name,
        "filename": part.filename,
        "content_type": part.content_type,
        "body_base64": STANDARD.encode(&part.body),
    })
}

fn response_json(response: &OCRResponse) -> Result<Value, Box<HelperError>> {
    let mut pages = Vec::with_capacity(response.pages.len());
    for page in &response.pages {
        let blocks = match page.extra.get("blocks") {
            Some(blocks) => serde_json::to_value(blocks).map_err(|error| failure(&error))?,
            None => Value::Null,
        };
        pages.push(json!({
            "index": page.index,
            "markdown": page.markdown,
            "blocks": blocks,
        }));
    }
    let usage = response.usage_info.as_ref().map_or(Value::Null, |usage| {
        json!({
            "pages_processed": usage.pages_processed,
            "credits": usage.credits,
        })
    });
    Ok(json!({
        "model": response.model,
        "pages": pages,
        "usage": usage,
    }))
}

fn ocr_failure(error: &OcrError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "OcrError".to_owned(),
        status: error.http_status_code(),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

pub(super) fn failure<Message>(message: Message) -> Box<HelperError>
where
    Message: std::fmt::Display,
{
    Box::new(HelperError {
        class: "B6806Oracle".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_string(),
    })
}
