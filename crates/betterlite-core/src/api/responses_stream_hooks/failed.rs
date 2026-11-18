use std::str::FromStr as _;

use betterlite_error::error::details::{MidStreamFallbackDetails, MidStreamStatus};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;

use super::ResponsesStreamHooks;
use super::billing::FailurePrice;
use super::events::nested_response;
use super::state::ResponsesStreamHookState;
use crate::api::response_synthetic_events::billing::{UsageSource, usage_as_model};
use crate::api::responses_stream_primitives::error_fields::StreamErrorObject;
use crate::api::responses_stream_primitives::failure::{
    map_stream_error_to_exception, mid_stream_fallback_eligible,
};
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;
use crate::api::responses_usage::{UsageInput, normalize_and_project};

#[derive(Debug, Error)]
#[error("responses stream failed")]
pub struct StreamFailure {
    pub exception: ErrorClass,
    pub original_exception: Option<ErrorClass>,
}

impl ResponsesStreamHooks<'_> {
    pub async fn handle_logging_failed_response<C>(
        &self,
        state: &mut ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
        details: &mut IndexMap<String, Json>,
        calculator: Option<&C>,
    ) where
        C: FailurePrice,
    {
        let response = state.completed_response.as_ref().and_then(nested_response);
        let error = response.and_then(|response| response.error.as_ref());
        record_failed_response_usage(response, details, calculator).await;
        let mapped = map_response_error(
            &error,
            &state.model,
            state.custom_llm_provider.as_deref().unwrap_or_default(),
            self.redactor,
        );
        self.handle_failure(state, callback, &mapped).await;
    }

    pub fn maybe_raise_for_error_event(
        &self,
        state: &ResponsesStreamBaseState,
        event: &ResponsesAPIStreamingResponse,
    ) -> Result<(), StreamFailure> {
        let Some(mapped) = mapped_event_error(
            event,
            &state.model,
            state.custom_llm_provider.as_deref().unwrap_or_default(),
            self.redactor,
        ) else {
            return Ok(());
        };
        let status = mapped.metadata().status_code;
        if !mid_stream_fallback_eligible(&mapped, Some(status)) {
            return Err(StreamFailure {
                exception: mapped,
                original_exception: None,
            });
        }
        let context = ErrorContext {
            message: exception_message(&mapped),
            model: Some(state.model.clone()),
            provider: Some(state.custom_llm_provider.clone().unwrap_or_default()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        };
        let exception = ErrorClass::MidStreamFallback(Box::new(MidStreamFallbackDetails {
            context,
            original_status: MidStreamStatus(status),
            generated_content: state.generated_content.clone(),
            is_pre_first_chunk: !state.yielded_first_chunk,
        }));
        Err(StreamFailure {
            exception,
            original_exception: Some(mapped),
        })
    }
}

async fn record_failed_response_usage<C>(
    response: Option<&ResponsesApiResponse>,
    details: &mut IndexMap<String, Json>,
    calculator: Option<&C>,
) where
    C: FailurePrice,
{
    let Some(response) = response else {
        return;
    };
    let Some(usage) = usage_as_model(response.usage.as_ref().map(UsageSource::Model)) else {
        return;
    };
    match normalize_and_project(UsageInput::Response(&usage)) {
        Ok(projected) => {
            if let Ok(combined) = serde_json::from_value::<Json>(projected) {
                details.insert("combined_usage_object".to_owned(), combined);
            }
        }
        Err(error) => {
            tracing::debug!("could not record usage for failed responses stream: {error}");
            return;
        }
    }
    let cost = match calculator {
        Some(calculator) => calculator.cost(response).await.unwrap_or(Decimal::ZERO),
        None => Decimal::ZERO,
    };
    if let Some(recorded) = decimal_json(cost) {
        details.insert("response_cost".to_owned(), recorded);
    }
}

fn decimal_json(cost: Decimal) -> Option<Json> {
    match serde_json::Number::from_str(&cost.to_string()) {
        Ok(number) => Some(Json::Number(number)),
        Err(_) => None,
    }
}

fn map_response_error<T: Serialize>(
    error: &T,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> ErrorClass {
    let fields = serde_json::to_value(error)
        .and_then(serde_json::from_value::<IndexMap<String, Json>>)
        .unwrap_or_default();
    map_stream_error_to_exception(
        StreamErrorObject::Mapping(&fields),
        model,
        provider,
        redactor,
    )
}

fn mapped_event_error(
    event: &ResponsesAPIStreamingResponse,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> Option<ErrorClass> {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => {
            if let KnownResponseEvent::Error(record) = known.as_ref() {
                Some(map_response_error(&record.error, model, provider, redactor))
            } else if let KnownResponseEvent::ResponseFailed(record) = known.as_ref() {
                Some(map_response_error(
                    &record.response.error,
                    model,
                    provider,
                    redactor,
                ))
            } else {
                None
            }
        }
        ResponsesAPIStreamingResponse::Generic(record) => match record.r#type.as_str() {
            "error" => Some(map_response_error(
                &record.extra.get("error"),
                model,
                provider,
                redactor,
            )),
            "response.failed" => Some(map_response_error(&Json::Null, model, provider, redactor)),
            _ => None,
        },
    }
}

fn exception_message(class: &ErrorClass) -> String {
    let Ok(value) = serde_json::to_value(class) else {
        return String::new();
    };
    value
        .pointer("/context/message")
        .or_else(|| value.pointer("/message"))
        .and_then(|found| found.as_str())
        .unwrap_or_default()
        .to_owned()
}
