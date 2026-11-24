use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_error::error::details::{
    ApiDetails, RateLimitDetails, ResponseDetails, TimeoutDetails, TimeoutStatus,
};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::Json;
use betterlite_ocr::json::{Json as OcrJson, Object as OcrObject};
use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::ocr_http::OcrClient;
use betterlite_ocr::ocr_input::LiteLLMOcrRequest;
use betterlite_ocr::ocr_input::wire::{OcrWireRequest, decode_request_input};
use betterlite_ocr::ocr_route::aocr;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;

use super::seams::OcrProbeCall;
use crate::health_probe_modes::calls::{HealthProbeReply, OcrCall};
use crate::health_probe_modes::error::HealthProbeError;

const MODEL_KEY: &str = "model";
const API_BASE_KEY: &str = "api_base";
const PROVIDER_KEY: &str = "custom_llm_provider";
const TIMEOUT_KEY: &str = "timeout";
const EXTRA_HEADERS_KEY: &str = "extra_headers";
const PARAMETERS_KEY: &str = "parameters";

// the ocr mode arguments as the request B6809's aocr takes: its keywords split out, the rest as optional params
pub fn aocr_request(call: OcrCall) -> Result<LiteLLMOcrRequest, HealthProbeError> {
    let mut parameters = ocr_parameters(&call.parameters)?;
    let model_value = parameters.shift_remove(MODEL_KEY).unwrap_or(OcrJson::Null);
    let encoded = serde_json::to_string(&model_value)
        .map_err(|source| parameter_failure(MODEL_KEY, source))?;
    let model =
        serde_json::from_str(&encoded).map_err(|source| parameter_failure(MODEL_KEY, source))?;
    let api_base = take_optional(&mut parameters, API_BASE_KEY)?;
    let custom_llm_provider = take_optional(&mut parameters, PROVIDER_KEY)?;
    let timeout_seconds = take_optional(&mut parameters, TIMEOUT_KEY)?;
    let extra_headers = take_optional(&mut parameters, EXTRA_HEADERS_KEY)?;
    let wire = OcrWireRequest {
        model,
        document: call.document,
        api_key: call.api_key,
        api_base,
        custom_llm_provider,
        extra_headers,
        optional_params: parameters,
        input_sources: BTreeMap::new(),
        timeout_seconds,
    };
    decode_request_input(wire).map_err(ocr_failure)
}

// the production OcrProbeCall: one aocr call over a client built for it, its response headers carried as they are
pub struct OcrProbe<F> {
    pub client: F,
}

#[async_trait]
impl<F> OcrProbeCall for OcrProbe<F>
where
    F: Fn() -> OcrClient + Send + Sync,
{
    async fn ocr(&self, call: OcrCall) -> Result<HealthProbeReply, HealthProbeError> {
        let request = aocr_request(call)?;
        let client = (self.client)();
        let _response = aocr(client, request).await.map_err(ocr_failure)?;
        // LiteLLM's ocr route never fills _hidden_params with headers, and OCRResponse holds none
        Ok(HealthProbeReply::default())
    }
}

fn ocr_parameters(parameters: &IndexMap<String, Json>) -> Result<OcrObject, HealthProbeError> {
    let encoded = serde_json::to_string(parameters)
        .map_err(|source| parameter_failure(PARAMETERS_KEY, source))?;
    serde_json::from_str(&encoded).map_err(|source| parameter_failure(PARAMETERS_KEY, source))
}

fn take_optional<T>(
    parameters: &mut OcrObject,
    key: &'static str,
) -> Result<Option<T>, HealthProbeError>
where
    T: DeserializeOwned,
{
    let Some(value) = parameters.shift_remove(key) else {
        return Ok(None);
    };
    if matches!(value, OcrJson::Null) {
        return Ok(None);
    }
    let encoded = serde_json::to_string(&value).map_err(|source| parameter_failure(key, source))?;
    serde_json::from_str(&encoded)
        .map(Some)
        .map_err(|source| parameter_failure(key, source))
}

fn parameter_failure(key: &str, source: serde_json::Error) -> HealthProbeError {
    HealthProbeError::ParameterValue {
        key: key.to_owned(),
        source,
    }
}

fn ocr_failure(error: OcrError) -> HealthProbeError {
    let message = error.to_string();
    let class = ocr_class(&message, error.http_status_code());
    HealthProbeError::Backend {
        class,
        message,
        source: Box::new(error),
    }
}

// the class LiteLLM's exception mapper raises for a native failure carrying this status
fn ocr_class(message: &str, status: Option<u16>) -> ErrorClass {
    let context = ErrorContext {
        message: String::from(message),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    let Some(code) = status else {
        return ErrorClass::ApiConnection(Box::new(context));
    };
    class_for_status(code, context)
}

fn class_for_status(code: u16, context: ErrorContext) -> ErrorClass {
    match code {
        401 => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, Some(401))))
        }
        403 => {
            ErrorClass::PermissionDenied(Box::new(ResponseDetails::with_status(context, Some(403))))
        }
        404 => ErrorClass::NotFound(Box::new(ResponseDetails::with_status(context, Some(404)))),
        408 => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(408),
        })),
        429 => ErrorClass::RateLimit(Box::new(RateLimitDetails {
            context,
            category: RateLimitCategory::VendorRateLimit,
            dimension: None,
            headers: BTreeMap::new(),
        })),
        500 => ErrorClass::InternalServer(Box::new(context)),
        502 => ErrorClass::BadGateway(Box::new(context)),
        503 => ErrorClass::ServiceUnavailable(Box::new(context)),
        504 => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(504),
        })),
        other if (400..=499).contains(&other) => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, Some(other))))
        }
        other => ErrorClass::Api(Box::new(ApiDetails {
            context,
            status: other,
        })),
    }
}
