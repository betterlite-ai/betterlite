use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use serde::Serialize;

use crate::datadog_llm_obs::span_records::{DDIntakePayload, DDSpanAttributes, LLMObsPayload};
use crate::datadog_logging::transport::{LOGGING_TIMEOUT, PostRequest, Transport};

use super::error::DatadogLlmObsError;
use super::settings::DatadogLlmObsSettings;
use super::tags::envelope_tags;

const API_KEY_HEADER: HeaderName = HeaderName::from_static("dd-api-key");
const ACCEPTED_STATUS: u16 = 202;

#[derive(Serialize)]
struct IntakeEnvelope {
    data: DDIntakePayload,
}

#[must_use]
pub fn intake_payload(
    settings: &DatadogLlmObsSettings,
    spans: Vec<LLMObsPayload>,
) -> DDIntakePayload {
    DDIntakePayload {
        kind: "span".to_owned(),
        attributes: DDSpanAttributes {
            ml_app: settings.environment.service.clone(),
            tags: envelope_tags(&settings.environment),
            spans,
        },
    }
}

// LiteLLM sends the spans as one uncompressed JSON document and accepts only 202
pub async fn send_batch(
    settings: &DatadogLlmObsSettings,
    transport: &dyn Transport,
    spans: Vec<LLMObsPayload>,
) -> Result<(), DatadogLlmObsError> {
    let envelope = IntakeEnvelope {
        data: intake_payload(settings, spans),
    };
    let body = serde_json::to_vec(&envelope).map_err(DatadogLlmObsError::Encode)?;
    let request = PostRequest {
        url: settings.intake_url.clone(),
        headers: headers(settings)?,
        body,
        timeout: LOGGING_TIMEOUT,
    };
    let response = transport.post(request).await?;
    if response.status == ACCEPTED_STATUS {
        return Ok(());
    }
    let detail = str::from_utf8(&response.body)
        .unwrap_or("<non-UTF-8 body>")
        .to_owned();
    Err(DatadogLlmObsError::UnexpectedStatus {
        status: response.status,
        detail,
    })
}

fn headers(settings: &DatadogLlmObsSettings) -> Result<HeaderMap, DatadogLlmObsError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let Some(api_key) = &settings.api_key else {
        return Ok(headers);
    };
    let mut value =
        HeaderValue::from_str(api_key.expose_secret()).map_err(DatadogLlmObsError::ApiKeyHeader)?;
    value.set_sensitive(true);
    headers.insert(API_KEY_HEADER, value);
    Ok(headers)
}
