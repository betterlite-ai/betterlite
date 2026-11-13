use crate::json::{Json, Object};
use crate::ocr_config::decode::{OcrFieldKind, decode_and_normalize_response, decode_value};
use crate::ocr_config::{
    OCR_INLINE_MAX_BYTES, OcrProviderConfig, OcrTransform, PreparedOcrRequest,
};
use crate::ocr_document::{InlineDocument, OcrDocument};
use crate::ocr_error::OcrError;
use crate::ocr_http::{OcrClient, body_document};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use std::collections::BTreeMap;
use std::future::{Future, ready};

mod headers;
mod lax;
pub mod options;
pub mod response;
mod tests;
mod url;
pub mod wire;

pub const COHERE_PARSE_API_BASE: &str = "https://api.cohere.com";
pub const COHERE_API_KEY_ENV: &str = "COHERE_API_KEY";
pub const COHERE_HEALTH_CHECK_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC";

#[derive(Default)]
pub struct CohereParseConfig;

impl OcrProviderConfig for CohereParseConfig {
    type OcrParams = options::CohereOptions;
    type ProviderRequest = wire::CohereRequest;
    type Environment = Vec<(String, String)>;

    fn get_supported_ocr_params(&self, _model: &str) -> &'static [&'static str] {
        &["output_format", "req_format"]
    }

    fn get_api_key_env_var(&self) -> Option<&'static str> {
        Some(COHERE_API_KEY_ENV)
    }

    fn secret_names(&self) -> Vec<&'static str> {
        vec![COHERE_API_KEY_ENV]
    }

    fn get_health_check_document(&self) -> OcrDocument {
        OcrDocument::ImageUrl {
            image_url: COHERE_HEALTH_CHECK_DATA_URI.to_owned(),
            extra: BTreeMap::new(),
        }
    }

    fn map_ocr_params(
        &self,
        non_default_params: &Object,
        _model: &str,
    ) -> Result<Self::OcrParams, OcrError> {
        decode_value(
            &Json::Object(non_default_params.clone()),
            "optional_params",
            OcrFieldKind::Request,
        )
    }

    fn validate_environment(
        &self,
        request: &PreparedOcrRequest,
        _client: &OcrClient,
    ) -> impl Future<Output = Result<Self::Environment, OcrError>> + Send {
        ready(headers::resolve(request))
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        _optional_params: &Self::OcrParams,
        _environment: &Self::Environment,
    ) -> Result<String, OcrError> {
        let api_base = request
            .connection
            .api_base
            .as_deref()
            .unwrap_or(COHERE_PARSE_API_BASE);
        url::complete_parse_url(api_base)
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        let image_url = image_url(document)?;
        Ok(wire::build_request(
            transform.model,
            image_url,
            transform.optional_params,
        ))
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
            response::normalize_response,
        )
    }

    fn validate_request_body(&self, body: &Json) -> Result<(), OcrError> {
        validate_document(&body_document(body)?)
    }
}

pub fn validate_document(document: &OcrDocument) -> Result<(), OcrError> {
    let OcrDocument::ImageUrl { image_url, .. } = document else {
        return Err(OcrError::CohereImageOnly);
    };
    if image_url.is_empty() {
        return Err(OcrError::CohereImageOnly);
    }
    let Some(inline) = InlineDocument::parse(image_url)? else {
        return Ok(());
    };
    if !inline.0.mime_type().type_.eq_ignore_ascii_case("image") {
        return Err(OcrError::CohereImageOnly);
    }
    inline.decode(OCR_INLINE_MAX_BYTES)?;
    Ok(())
}

fn image_url(document: OcrDocument) -> Result<String, OcrError> {
    validate_document(&document)?;
    let OcrDocument::ImageUrl { image_url, .. } = document else {
        return Err(OcrError::CohereImageOnly);
    };
    Ok(image_url)
}
