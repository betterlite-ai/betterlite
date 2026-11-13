use std::future::Future;

use serde::Serialize;
use serde_json::Number;

use crate::json::{Json, Object};
use crate::ocr_config::decode::decode_and_normalize_response;
use crate::ocr_config::{OcrProviderConfig, OcrTransform, PreparedOcrRequest};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrError, OcrParamsError};
use crate::ocr_http::OcrClient;
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use crate::vertex_ocr::VertexOcrEnvironment;
use crate::vertex_ocr::common;

mod normalize;
mod tests;
mod url;

const MODEL_PREFIX: &str = "deepseek-ai/";
const DEEPSEEK_OCR_PARAMS: [&str; 6] =
    ["stream", "temperature", "max_tokens", "top_p", "n", "stop"];
const DEFAULT_TEMPERATURE: f64 = 0.0_f64;
const DEFAULT_REPETITION_PENALTY: f64 = 1.05_f64;

#[derive(Clone, Debug, Serialize)]
pub struct DeepSeekOcrRequest {
    pub model: String,
    pub messages: Vec<DeepSeekOcrMessage>,
    #[serde(flatten)]
    pub params: Object,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeepSeekOcrMessage {
    pub role: UserRole,
    pub content: Vec<DeepSeekDocument>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum DeepSeekDocument {
    #[serde(rename = "image_url")]
    ImageUrl { image_url: String },
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    User,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VertexAIDeepSeekOCRConfig;

impl OcrProviderConfig for VertexAIDeepSeekOCRConfig {
    type Environment = VertexOcrEnvironment;
    type OcrParams = Object;
    type ProviderRequest = DeepSeekOcrRequest;

    fn get_api_key_env_var(&self) -> Option<&'static str> {
        Some(common::VERTEX_AI_API_KEY)
    }

    fn secret_names(&self) -> Vec<&'static str> {
        common::SECRET_NAMES.to_vec()
    }

    // the empty params make Ok(Default::default()) the same value as Ok(Object::new())
    #[cfg_attr(test, mutants::skip)]
    fn map_ocr_params(
        &self,
        _non_default_params: &Object,
        _model: &str,
    ) -> Result<Object, OcrError> {
        Ok(Object::new())
    }

    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        client: &OcrClient,
    ) -> impl Future<Output = Result<VertexOcrEnvironment, OcrError>> + Send {
        common::environment(request, client)
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        _optional_params: &Object,
        environment: &VertexOcrEnvironment,
    ) -> Result<String, OcrError> {
        let config = common::vertex_config(request)?;
        let location = common::location(request, &config);
        url::chat_completions_url(
            request.connection.api_base.as_deref(),
            &environment.project_id,
            &location,
        )
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Object>,
    ) -> Result<DeepSeekOcrRequest, OcrError> {
        if document.source().is_empty() {
            return Err(OcrError::MissingDocumentUrl);
        }
        let mut params = Object::new();
        params.insert("temperature".to_owned(), json_number(DEFAULT_TEMPERATURE)?);
        params.insert(
            "repetition_penalty".to_owned(),
            json_number(DEFAULT_REPETITION_PENALTY)?,
        );
        for (name, value) in transform.optional_params {
            if DEEPSEEK_OCR_PARAMS.contains(&name.as_str()) {
                params.insert(name.clone(), value.clone());
            }
        }
        Ok(DeepSeekOcrRequest {
            model: provider_model(transform.model)?,
            messages: vec![DeepSeekOcrMessage {
                role: UserRole::User,
                content: vec![DeepSeekDocument::ImageUrl {
                    image_url: document.source().to_owned(),
                }],
            }],
            params,
        })
    }

    fn transform_ocr_response(
        &self,
        model: &str,
        raw_response: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        decode_and_normalize_response(
            model,
            raw_response,
            request_format,
            normalize::normalize_response,
        )
    }
}

fn provider_model(model: &str) -> Result<String, OcrError> {
    let local_model = model.trim_start_matches(MODEL_PREFIX);
    if local_model.is_empty() {
        return Err(OcrError::RequestField {
            path: "model".to_owned(),
        });
    }
    Ok(format!("{MODEL_PREFIX}{local_model}"))
}

fn json_number(value: f64) -> Result<Json, OcrError> {
    Number::from_f64(value)
        .map(Json::Number)
        .ok_or_else(|| OcrError::Params(OcrParamsError::Body))
}
