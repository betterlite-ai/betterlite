use super::{LiteLLMOcrRequest, OcrConnectionInputs, OcrDocumentInput};
use crate::json::{Json, Object};
use crate::ocr_config::connection::OcrInputSource;
use crate::ocr_config::decode::{OcrFieldKind, decode_value};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrWireRequest<D = Json> {
    pub model: String,
    pub document: D,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub extra_headers: Option<Object>,
    #[serde(default)]
    pub optional_params: Object,
    #[serde(default)]
    pub input_sources: BTreeMap<String, String>,
    pub timeout_seconds: Option<f64>,
}

pub fn decode_request(wire: OcrWireRequest<Json>) -> Result<LiteLLMOcrRequest, OcrError> {
    let document = decode_document(&wire.document)?;
    decode_request_input(OcrWireRequest {
        model: wire.model,
        document,
        api_key: wire.api_key,
        api_base: wire.api_base,
        custom_llm_provider: wire.custom_llm_provider,
        extra_headers: wire.extra_headers,
        optional_params: wire.optional_params,
        input_sources: wire.input_sources,
        timeout_seconds: wire.timeout_seconds,
    })
}

pub fn decode_request_input<D>(wire: OcrWireRequest<D>) -> Result<LiteLLMOcrRequest, OcrError>
where
    OcrDocumentInput: From<D>,
{
    let timeout = wire
        .timeout_seconds
        .map(|seconds| {
            Duration::try_from_secs_f64(seconds).map_err(|_source| OcrError::RequestField {
                path: "timeout_seconds".to_owned(),
            })
        })
        .transpose()?;
    let mut input_sources = BTreeMap::new();
    for (name, value) in wire.input_sources {
        let source = match value.as_str() {
            "request" => OcrInputSource::Request,
            "deployment" => OcrInputSource::Deployment,
            "environment" => OcrInputSource::Environment,
            _ => {
                return Err(OcrError::RequestField {
                    path: format!("input_sources.{name}"),
                });
            }
        };
        let _previous = input_sources.insert(name, source);
    }
    LiteLLMOcrRequest::from_inputs(
        &wire.model,
        OcrDocumentInput::from(wire.document),
        wire.custom_llm_provider.as_deref(),
        wire.optional_params,
        OcrConnectionInputs {
            api_key: wire.api_key,
            api_base: wire.api_base,
            extra_headers: wire.extra_headers.unwrap_or_default(),
            timeout,
            input_sources,
        },
    )
}

pub fn decode_document(value: &Json) -> Result<OcrDocument, OcrError> {
    let kind = match value {
        Json::Object(fields) => match fields.get("type") {
            Some(Json::Text(text)) => Some(text.as_str()),
            Some(_) | None => None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    };
    let missing_url = match kind {
        Some("document_url") => {
            !matches!(value, Json::Object(fields) if fields.contains_key("document_url"))
        }
        Some("image_url") => {
            !matches!(value, Json::Object(fields) if fields.contains_key("image_url"))
        }
        _ => false,
    };
    if missing_url {
        return Err(OcrError::MissingDocumentUrl);
    }
    decode_value(value, "document", OcrFieldKind::Request)
}
