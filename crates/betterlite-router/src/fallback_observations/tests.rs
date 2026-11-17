#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::{
    BudgetExceededDetails, GuardrailDetails, RateLimitDetails, ResponseDetails,
    SchemaValidationDetails,
};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};

use super::error_info::{FALLBACK_ERRORS_HEADER, FallbackErrorInfo, exception_message};
use super::observer::{FallbackEvent, FallbackObservationError, FallbackObserver};
use crate::router_core::headers::{Response, ensure_additional_headers_dict};

mod fallback_observations_behaviors;
mod fallback_observations_boundaries;

pub(crate) const MODEL: &str = "gpt-4";

pub(crate) const BUDGET_MESSAGE: &str =
    "Budget has been exceeded! Current cost: 1.5, Max budget: 10";

pub(crate) const SCHEMA_MESSAGE: &str = r#"litellm.JSONSchemaValidationError: model=gpt-4, returned an invalid response={"id": "x"}, for schema={"type": "object"}.
Access raw response with `e.raw_response`"#;

pub(crate) fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: Some(MODEL.to_owned()),
        provider: Some("openai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

pub(crate) fn rate_limit(message: &str, body_param: Option<&str>) -> ErrorClass {
    let mut details = context(message);
    details.body_param = body_param.map(str::to_owned);
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: details,
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

pub(crate) fn authentication(message: &str) -> ErrorClass {
    ErrorClass::Authentication(Box::new(ResponseDetails::from(context(message))))
}

pub(crate) fn bad_request(
    message: &str,
    body_type: Option<&str>,
    body_param: Option<&str>,
) -> ErrorClass {
    let mut details = context(message);
    details.body_type = body_type.map(str::to_owned);
    details.body_param = body_param.map(str::to_owned);
    ErrorClass::BadRequest(Box::new(ResponseDetails::from(details)))
}

pub(crate) fn budget(message: Option<&str>) -> ErrorClass {
    ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::new(15, 1),
        max_budget: Decimal::new(10, 0),
        message: message.map(str::to_owned),
        provider: "openai".to_owned(),
        entity_type: None,
        entity_id: None,
        status_code: 422,
    }))
}

pub(crate) fn guardrail(message: &str) -> ErrorClass {
    ErrorClass::GuardrailRaised(Box::new(GuardrailDetails {
        guardrail_name: Some("pii".to_owned()),
        message: message.to_owned(),
        status_code: 400,
        blocked_content: false,
    }))
}

pub(crate) fn unsupported(message: &str) -> ErrorClass {
    ErrorClass::UnsupportedParams(Box::new(context(message)))
}

pub(crate) fn schema() -> ErrorClass {
    ErrorClass::JsonSchemaValidation(Box::new(SchemaValidationDetails {
        model: MODEL.to_owned(),
        provider: "openai".to_owned(),
        raw_response: r#"{"id": "x"}"#.to_owned(),
        schema: r#"{"type": "object"}"#.to_owned(),
    }))
}

pub(crate) fn plain(message: &str) -> ErrorClass {
    ErrorClass::Unclassified(Box::new(context(message)))
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

pub(crate) fn info(
    message: &str,
    error_type: &str,
    param: Option<&str>,
    code: Option<&str>,
) -> FallbackErrorInfo {
    FallbackErrorInfo {
        message: message.to_owned(),
        error_type: error_type.to_owned(),
        param: param.map(str::to_owned),
        code: code.map(str::to_owned),
    }
}

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn object(source: &str) -> JsonObject {
    match json(source) {
        Json::Object(object) => object,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("{source} must parse to a JSON object")
        }
    }
}

pub(crate) fn request() -> JsonObject {
    object(r#"{"model": "gpt-4", "metadata": {"team": "payments"}}"#)
}

fn hidden_params(response: &Response) -> JsonObject {
    match response {
        Response::Object(body) => hidden_of(body),
        Response::Attributed(hidden) => hidden.clone(),
        Response::Absent | Response::Stream | Response::Opaque => JsonObject::new(),
    }
}

fn hidden_of(body: &JsonObject) -> JsonObject {
    let Some(Json::Object(hidden)) = body.get("_hidden_params") else {
        return JsonObject::new();
    };
    hidden.clone()
}

pub(crate) fn additional_headers(response: &Response) -> JsonObject {
    ensure_additional_headers_dict(&hidden_params(response))
}

pub(crate) fn header(response: &Response, name: &str) -> Option<Json> {
    additional_headers(response).get(name).cloned()
}

pub(crate) fn errors_header(response: &Response) -> Json {
    let Some(Json::Text(text)) = header(response, FALLBACK_ERRORS_HEADER) else {
        return Json::Null;
    };
    serde_json::from_str(&text).unwrap()
}

pub(crate) struct RecordingObserver {
    pub(crate) index: u32,
    pub(crate) calls: Arc<Mutex<Vec<String>>>,
}

impl RecordingObserver {
    pub(crate) fn new(index: u32, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { index, calls }
    }

    fn record(&self, outcome: &str, event: FallbackEvent<'_>) {
        let index = self.index;
        let group = event.original_model_group;
        let message = exception_message(event.exception);
        let model = match event.request.get("model") {
            Some(Json::Text(model)) => model.as_str(),
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => "",
        };
        self.calls
            .lock()
            .unwrap()
            .push(format!("{index}:{outcome}:{group}:{model}:{message}"));
    }
}

#[async_trait]
impl FallbackObserver for RecordingObserver {
    async fn log_success_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        self.record("success", event);
        Ok(())
    }

    async fn log_failure_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        self.record("failure", event);
        Ok(())
    }
}

pub(crate) struct FailingObserver {
    pub(crate) message: String,
}

#[async_trait]
impl FallbackObserver for FailingObserver {
    async fn log_success_fallback_event(
        &self,
        _event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        Err(FallbackObservationError::Observer {
            message: self.message.clone(),
        })
    }

    async fn log_failure_fallback_event(
        &self,
        _event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        Err(FallbackObservationError::Observer {
            message: self.message.clone(),
        })
    }
}

pub(crate) struct SilentObserver;

impl FallbackObserver for SilentObserver {}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub(crate) fn messages(&self) -> Vec<String> {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|line| {
                line.split_once(": ")
                    .map_or(line, |(_target, message)| message)
                    .to_owned()
            })
            .collect()
    }
}

pub(crate) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
