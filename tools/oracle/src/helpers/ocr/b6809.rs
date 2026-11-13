mod server;

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_ocr::json::{Json, Object};
use betterlite_ocr::ocr_auth::{
    AwsRequestSigner, AwsSigningRequest, AzureTokenSource, VertexAccessToken, VertexTokenSource,
};
use betterlite_ocr::ocr_config::PreparedOcrRequest;
use betterlite_ocr::ocr_config::connection::OcrSourced;
use betterlite_ocr::ocr_error::{OcrError, OcrSecretError};
use betterlite_ocr::ocr_http::{OcrClient, SecretSource};
use betterlite_ocr::ocr_input::wire::{OcrWireRequest, decode_request};
use betterlite_ocr::ocr_route::aocr;
use betterlite_ocr::ocr_settings::OcrSettings;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use server::{Captured, Server};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    call: Option<Call>,
    #[serde(default)]
    decode: Vec<WireInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    model: String,
    document: Json,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base_path: String,
    #[serde(default)]
    extra_headers: Option<Object>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    timeout_seconds: Option<f64>,
    answer: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireInput {
    name: String,
    model: String,
    document: Json,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    extra_headers: Option<Object>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    timeout_seconds: Option<f64>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        if let Some(call) = &input.call {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| failure(&error))?;
            return runtime.block_on(run_call(call));
        }
        Ok(run_decode(&input.decode))
    }
}

struct Unused;

impl SecretSource for Unused {
    fn secret(&self, _name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        Ok(None)
    }
}

#[async_trait]
impl AzureTokenSource for Unused {
    async fn token(
        &self,
        _request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        Ok(None)
    }
}

#[async_trait]
impl AwsRequestSigner for Unused {
    async fn sign(
        &self,
        _request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl VertexTokenSource for Unused {
    async fn token(&self, _request: &PreparedOcrRequest) -> Result<VertexAccessToken, OcrError> {
        Err(OcrError::Unsupported("vertex tokens in the oracle"))
    }
}

fn client() -> Result<OcrClient, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| failure(&error))?,
    );
    let http = ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT)
        .map_err(|error| failure(&error))?;
    let validator = UrlValidator::new(
        SystemResolver,
        &["127.0.0.1".to_owned()],
        &Verification::Enabled,
    );
    Ok(OcrClient {
        provider_http: http.clone(),
        polling_http: http.clone(),
        document_http: http,
        document_fetcher: UserUrlFetcher {
            validator,
            validation: true,
        },
        settings: OcrSettings::default(),
        secrets: Arc::new(Unused),
        azure: Arc::new(Unused),
        aws: Arc::new(Unused),
        vertex: Arc::new(Unused),
    })
}

async fn run_call(call: &Call) -> HelperResult {
    let answer = serde_json::to_vec(&call.answer).map_err(|error| failure(&error))?;
    let server = Server::start(answer).await?;
    let wire = OcrWireRequest {
        model: call.model.clone(),
        document: call.document.clone(),
        api_key: call
            .api_key
            .as_ref()
            .map(|key| SecretString::from(key.as_str())),
        api_base: Some(format!("{}{}", server.base, call.api_base_path)),
        custom_llm_provider: None,
        extra_headers: call.extra_headers.clone(),
        optional_params: call.optional_params.clone(),
        input_sources: BTreeMap::new(),
        timeout_seconds: call.timeout_seconds,
    };
    let request = decode_request(wire).map_err(|error| ocr_failure(&error))?;
    let response = aocr(client()?, request)
        .await
        .map_err(|error| ocr_failure(&error))?;
    let captured = server.finish()?;
    observed(&captured, &response)
}

fn observed(captured: &Captured, response: &betterlite_ocr::response::OCRResponse) -> HelperResult {
    let body: Value = serde_json::from_slice(&captured.body).map_err(|error| failure(&error))?;
    let response_json = serde_json::to_value(response).map_err(|error| failure(&error))?;
    Ok(json!({
        "url_path": captured.path,
        "authorization": captured.headers.get("authorization"),
        "content_type": captured.headers.get("content-type"),
        "body": body,
        "response": response_json,
    }))
}

fn run_decode(attempts: &[WireInput]) -> Value {
    let mut results = Vec::with_capacity(attempts.len());
    for attempt in attempts {
        let wire = OcrWireRequest {
            model: attempt.model.clone(),
            document: attempt.document.clone(),
            api_key: attempt
                .api_key
                .as_ref()
                .map(|key| SecretString::from(key.as_str())),
            api_base: attempt.api_base.clone(),
            custom_llm_provider: attempt.custom_llm_provider.clone(),
            extra_headers: attempt.extra_headers.clone(),
            optional_params: attempt.optional_params.clone(),
            input_sources: BTreeMap::new(),
            timeout_seconds: attempt.timeout_seconds,
        };
        let result = match decode_request(wire) {
            Ok(_request) => json!({"name": attempt.name, "ok": true}),
            Err(error) => json!({
                "name": attempt.name,
                "ok": false,
                "status": error.http_status_code(),
            }),
        };
        results.push(result);
    }
    json!({"decode": results})
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
        class: "B6809Oracle".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_string(),
    })
}
