use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_ocr::json::{Json as OcrJson, Object as OcrObject};
use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::ocr_http::OcrClient;
use betterlite_ocr::ocr_input::LiteLLMOcrRequest;
use betterlite_ocr::ocr_input::wire::{OcrWireRequest, decode_request_input};
use betterlite_ocr::ocr_route::aocr;
use betterlite_sdk::call_wrapper::error::plain_error;
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
    let model = take_required(&mut parameters, MODEL_KEY)?;
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

fn take_required<T>(parameters: &mut OcrObject, key: &'static str) -> Result<T, HealthProbeError>
where
    T: DeserializeOwned,
{
    let value = parameters.shift_remove(key).unwrap_or(OcrJson::Null);
    convert(key, &value)
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
    convert(key, &value).map(Some)
}

fn convert<T>(key: &str, value: &OcrJson) -> Result<T, HealthProbeError>
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_string(value).map_err(|source| parameter_failure(key, source))?;
    serde_json::from_str(&encoded).map_err(|source| parameter_failure(key, source))
}

fn parameter_failure(key: &str, source: serde_json::Error) -> HealthProbeError {
    HealthProbeError::ParameterValue {
        key: key.to_owned(),
        source,
    }
}

fn ocr_failure(error: OcrError) -> HealthProbeError {
    let message = error.to_string();
    HealthProbeError::Backend {
        class: plain_error(&message),
        message,
        source: Box::new(error),
    }
}
