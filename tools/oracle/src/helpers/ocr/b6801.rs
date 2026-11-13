mod support;

use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_ocr::json::{Json, Object};
use betterlite_ocr::mistral_ocr::MistralOcrConfig;
use betterlite_ocr::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use betterlite_ocr::ocr_config::decode::response_format;
use betterlite_ocr::ocr_config::{
    OCR_RESPONSE_MAX_BYTES, OcrProviderConfig as _, PreparedOcrRequest, Secrets,
};
use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::ocr_http::{CallHooks, OcrWireRequest};
use betterlite_ocr::ocr_settings::OcrSettings;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use support::fail;

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
    extra_headers: Object,
    #[serde(default)]
    api_base_path: String,
    #[serde(default)]
    api_base_query: String,
    answer: Value,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let request = prepared_request(input)?;
        let outbound = support::runtime()?.block_on(async {
            let client = support::client()?;
            MistralOcrConfig
                .prepare_request(&request, &client, &NoHooks)
                .await
                .map_err(|error| ocr_failure(&error))
        })?;
        result_value(input, outbound)
    }
}

fn result_value(input: &Input, outbound: OutboundRequest) -> HelperResult {
    let RequestBody::Json(bytes) = outbound.body else {
        return Err(fail("HelperError", "the Mistral body is JSON".to_owned()));
    };
    let body: Value =
        serde_json::from_slice(&bytes).map_err(|error| fail("HelperError", error.to_string()))?;
    let format = response_format(&input.options).map_err(|error| ocr_failure(&error))?;
    let answer = serde_json::to_vec(&input.answer)
        .map_err(|error| fail("HelperError", error.to_string()))?;
    let response = MistralOcrConfig
        .transform_ocr_response(&input.model, &answer, format)
        .map_err(|error| ocr_failure(&error))?;
    let url_path = match outbound.url.query() {
        Some(query) => format!("{}?{query}", outbound.url.path()),
        None => outbound.url.path().to_owned(),
    };
    let authorization = outbound
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let content_type = outbound
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    Ok(json!({
        "url_path": url_path,
        "authorization": authorization,
        "content_type": content_type,
        "body": body,
        "response": response,
    }))
}

fn prepared_request(input: &Input) -> Result<PreparedOcrRequest, Box<HelperError>> {
    let connection = OcrConnection::new(
        ResolvedOcrCredentials {
            api_key: input.api_key.as_ref().map(|key| OcrSourced {
                value: SecretString::from(key.as_str()),
                source: OcrInputSource::Request,
            }),
            api_base: Some(OcrSourced {
                value: format!(
                    "http://mock.invalid{}{}",
                    input.api_base_path, input.api_base_query
                ),
                source: OcrInputSource::Request,
            }),
        },
        OcrTransportConfig {
            extra_headers: header_pairs(&input.extra_headers)?,
            extra_headers_source: OcrInputSource::Deployment,
            timeout: None,
            max_response_bytes: OCR_RESPONSE_MAX_BYTES,
        },
        OcrSettings::default(),
        Secrets::new(),
    );
    Ok(PreparedOcrRequest {
        model: input.model.clone(),
        document: input.document.clone(),
        connection,
        caller_document: false,
        optional_params: input.options.clone(),
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    })
}

fn header_pairs(headers: &Object) -> Result<Vec<(String, String)>, Box<HelperError>> {
    headers
        .iter()
        .map(|(name, value)| match value {
            Json::Text(text) => Ok((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => Err(
                fail("HelperInput", format!("the header {name} is not a string")),
            ),
        })
        .collect()
}

fn ocr_failure(error: &OcrError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "OcrError".to_owned(),
        status: error.http_status_code(),
        error_type: None,
        param: None,
        code: None,
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
