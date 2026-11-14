use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_guardrails::model_armor::config::DEFAULT_LOCATION;
use betterlite_guardrails::model_armor::file_scanning::ByteDataType;
use betterlite_guardrails::model_armor::transport::error::ArmorTokenError;
use betterlite_guardrails::model_armor::transport::project::ResolvedProject;
use betterlite_guardrails::model_armor::transport::tokens::ArmorTokens;
use betterlite_guardrails::model_armor::transport::wire::{SanitizeBody, Source};
use betterlite_guardrails::model_armor::transport::{ModelArmorTransport, file_body, text_body};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

#[derive(Deserialize)]
pub(crate) struct SanitizeRequest {
    pub(crate) template_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) location: Option<String>,
    pub(crate) api_endpoint: Option<String>,
    pub(crate) resolved_project_id: Option<String>,
    requests: Vec<RequestRun>,
}

#[derive(Deserialize)]
struct RequestRun {
    source: Source,
    content: Option<String>,
    file: Option<FilePayload>,
}

#[derive(Deserialize)]
struct FilePayload {
    bytes_b64: String,
    byte_data_type: ByteDataType,
}

#[derive(Serialize)]
struct SanitizeOutput {
    requests: Vec<RunOutput>,
}

#[derive(Serialize)]
struct RunOutput {
    url: String,
    body: SanitizeBody,
}

struct UnusedTokens;

#[async_trait]
impl ArmorTokens for UnusedTokens {
    type Error = ArmorTokenError;

    async fn access_token(&self) -> Result<(SecretString, Option<String>), ArmorTokenError> {
        Err(ArmorTokenError {
            detail: "the b3733 helper builds URLs and bodies without a token".to_owned(),
        })
    }
}

pub(crate) fn sanitize(request: &SanitizeRequest) -> HelperResult {
    let transport = transport(request);
    transport.resolved_project.remember(
        request.project_id.as_deref(),
        request.resolved_project_id.as_deref(),
    );
    let mut requests = Vec::with_capacity(request.requests.len());
    for run in &request.requests {
        requests.push(RunOutput {
            url: transport.sanitize_url(run.source),
            body: body(run)?,
        });
    }
    serde_json::to_value(SanitizeOutput { requests }).or_value_error()
}

fn body(run: &RequestRun) -> Result<SanitizeBody, Box<HelperError>> {
    if let Some(file) = &run.file {
        let bytes = STANDARD.decode(&file.bytes_b64).or_value_error()?;
        return Ok(file_body(&bytes, file.byte_data_type, run.source));
    }
    let content = run
        .content
        .as_deref()
        .ok_or_else(|| value_error("each request carries content or a file"))?;
    Ok(text_body(content, run.source))
}

fn transport(request: &SanitizeRequest) -> ModelArmorTransport<UnusedTokens> {
    ModelArmorTransport {
        template_id: request.template_id.clone(),
        project_id: request.project_id.clone(),
        location: request
            .location
            .as_deref()
            .filter(|location| !location.is_empty())
            .unwrap_or(DEFAULT_LOCATION)
            .to_owned(),
        api_endpoint: request.api_endpoint.clone(),
        sanitize_error_detail: true,
        timeout: None,
        client: reqwest::Client::default(),
        tokens: Arc::new(UnusedTokens),
        resolved_project: ResolvedProject::default(),
    }
}

fn value_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_owned(),
    })
}
