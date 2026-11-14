use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use betterlite_http::user_url::join::urljoin;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::error::NomaError;
use super::request::{application_id, metadata, text};
use super::wire::{NomaContext, ScanPayload, ScanResponse};
use super::{DEFAULT_API_BASE, NomaScanPolicy};

const AIDR_ENDPOINT: &str = "/ai-dr/v2/prompt/scan";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
// LiteLLM's AsyncHTTPHandler falls back to its 600 second total timeout
const DEFAULT_SCAN_TIMEOUT: Duration = Duration::from_secs(600);

pub(super) struct ScanTarget<'target> {
    pub(super) user_auth: &'target BTreeMap<String, GuardrailValue>,
    pub(super) extra_data: &'target BTreeMap<String, GuardrailValue>,
    pub(super) llm_request_id: Option<&'target str>,
}

impl NomaScanPolicy {
    pub(super) async fn call_noma_api(
        &self,
        input_items: &[Box<RawValue>],
        request_data: &BTreeMap<String, GuardrailValue>,
        target: &ScanTarget<'_>,
    ) -> Result<ScanResponse, NomaError> {
        let request_call_id = text(request_data, "litellm_call_id")?;
        let headers = request_headers(self.settings.api_key.as_ref(), request_call_id)?;
        let noma_context = context(
            request_data,
            target,
            self.settings.application_id.as_deref(),
        )?;
        let payload = ScanPayload {
            input: input_items,
            context: noma_context,
        };
        let base = self
            .settings
            .api_base
            .as_deref()
            .unwrap_or(DEFAULT_API_BASE);
        let endpoint = urljoin(base, AIDR_ENDPOINT).map_err(|source| NomaError::Endpoint {
            api_base: base.to_owned(),
            source,
        })?;
        let mut response = self
            .client
            .post(endpoint)
            .headers(headers)
            .json(&payload)
            .timeout(self.settings.timeout.unwrap_or(DEFAULT_SCAN_TIMEOUT))
            .send()
            .await
            .map_err(|source| NomaError::Transport { source })?;
        let status = response.status();
        if status.is_client_error() || status.is_server_error() {
            return Err(NomaError::HttpStatus { status });
        }
        let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|error| match error {
                ReadError::TooLarge { limit } => NomaError::ResponseTooLarge { limit },
                ReadError::Failed(source) => NomaError::Transport { source },
            })?;
        ScanResponse::from_body(&body)
    }
}

fn request_headers(
    api_key: Option<&SecretString>,
    request_call_id: Option<&str>,
) -> Result<HeaderMap, NomaError> {
    let mut headers = HeaderMap::new();
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        let mut value = HeaderValue::from_str(&format!("Bearer {}", key.expose_secret())).map_err(
            |source| NomaError::Header {
                name: "Authorization",
                source,
            },
        )?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
    }
    if let Some(noma_request_id) = request_call_id {
        let value = HeaderValue::from_str(noma_request_id).map_err(|source| NomaError::Header {
            name: "X-Noma-Request-ID",
            source,
        })?;
        headers.insert(HeaderName::from_static("x-noma-request-id"), value);
    }
    Ok(headers)
}

fn context(
    request_data: &BTreeMap<String, GuardrailValue>,
    target: &ScanTarget<'_>,
    configured: Option<&str>,
) -> Result<NomaContext, NomaError> {
    let application = application_id(
        request_data,
        target.user_auth,
        target.extra_data,
        configured,
    )?;
    let ip_address = metadata(request_data)?
        .and_then(|metadata| metadata.get("requester_ip_address"))
        .filter(|value| !matches!(value, GuardrailValue::Null))
        .cloned()
        .unwrap_or(GuardrailValue::Null);
    let user_id = match text(target.user_auth, "user_email")? {
        Some(email) => GuardrailValue::Text(email.to_owned()),
        None => text(target.user_auth, "user_id")?.map_or(GuardrailValue::Null, |id| {
            GuardrailValue::Text(id.to_owned())
        }),
    };
    let session_id = text(request_data, "litellm_call_id")?.map_or(GuardrailValue::Null, |id| {
        GuardrailValue::Text(id.to_owned())
    });
    let request_id = target.llm_request_id.map_or(GuardrailValue::Null, |id| {
        GuardrailValue::Text(id.to_owned())
    });
    Ok(NomaContext {
        application_id: application,
        ip_address,
        user_id,
        session_id,
        request_id,
    })
}
