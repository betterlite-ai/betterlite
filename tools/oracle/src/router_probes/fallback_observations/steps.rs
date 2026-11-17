use std::collections::BTreeMap;
use std::str::FromStr;
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
use betterlite_router::fallback_observations::details::fallback_error_detail;
use betterlite_router::fallback_observations::error_info::{
    FALLBACK_ERRORS_HEADER, exception_message, fallback_error_info, fallback_errors_from_headers,
};
use betterlite_router::fallback_observations::headers::attach_fallback_headers;
use betterlite_router::fallback_observations::messages::{
    fallback_outcome_message, no_fallback_group_message,
};
use betterlite_router::fallback_observations::observer::{
    FallbackEvent, FallbackObservationError, FallbackObserver, FallbackOutcome, log_fallback_event,
};
use betterlite_router::router_core::headers::{Response, ensure_additional_headers_dict};
use rust_decimal::Decimal;

use super::wire::{
    DetailCase, DetailFound, EventCall, EventCase, EventFound, HeaderCase, HeaderFound, Input,
    ObserverKind, Outcome, Output, ResponseKind,
};
use crate::router_probes::error::OperationError;

const MODEL: &str = "gpt-4";
const PROVIDER: &str = "openai";

pub(super) async fn output(input: &Input) -> Result<Output, OperationError> {
    Ok(Output {
        details: details(&input.details)?,
        no_fallback: input
            .no_fallback
            .iter()
            .map(|case| no_fallback_group_message(&case.lookup_groups, &case.fallbacks))
            .collect(),
        outcomes: input
            .outcomes
            .iter()
            .map(|case| {
                fallback_outcome_message(
                    case.model_group.as_deref(),
                    case.fallback_model_group.as_deref(),
                    &case.fallback_failure_detail,
                )
            })
            .collect(),
        headers: input.headers.iter().map(header_found).collect(),
        events: events(&input.events).await,
    })
}

fn details(cases: &[DetailCase]) -> Result<Vec<DetailFound>, OperationError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|source| {
        OperationError::InvalidRequest {
            message: format!("the fallback detail redactor could not be built: {source}"),
        }
    })?;
    cases
        .iter()
        .map(|case| {
            let class = error_class(case)?;
            Ok(DetailFound {
                info: fallback_error_info(&class),
                detail: fallback_error_detail(&redactor, &class),
            })
        })
        .collect()
}

// the adapter builds the ErrorClass the router would classify for the exception the case names, message text included
fn error_class(case: &DetailCase) -> Result<ErrorClass, OperationError> {
    match case {
        DetailCase::RateLimit {
            message,
            body_param,
        } => Ok(rate_limit(message, body_param.as_deref())),
        DetailCase::BadRequest {
            message,
            body_type,
            body_param,
        } => Ok(bad_request(
            message,
            body_type.as_deref(),
            body_param.as_deref(),
        )),
        DetailCase::Budget {
            message,
            current_cost,
            max_budget,
        } => budget(message.as_deref(), current_cost, max_budget),
        DetailCase::Guardrail {
            message,
            guardrail_name,
        } => Ok(guardrail(message, guardrail_name.as_deref())),
        DetailCase::JsonSchema {
            raw_response,
            json_schema,
        } => Ok(json_schema(raw_response, json_schema)),
        DetailCase::Plain { message, .. } => Ok(ErrorClass::Unclassified(Box::new(context(
            message.to_owned(),
        )))),
    }
}

fn context(message: String) -> ErrorContext {
    ErrorContext {
        message,
        model: Some(MODEL.to_owned()),
        provider: Some(PROVIDER.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn rate_limit(message: &str, body_param: Option<&str>) -> ErrorClass {
    // RateLimitError.__init__ puts its class name in front of the message it was given
    let mut details = context(format!("litellm.RateLimitError: {message}"));
    details.body_param = body_param.map(str::to_owned);
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: details,
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

fn bad_request(message: &str, body_type: Option<&str>, body_param: Option<&str>) -> ErrorClass {
    // BadRequestError.__init__ does the same with its own class name
    let mut details = context(format!("litellm.BadRequestError: {message}"));
    details.body_type = body_type.map(str::to_owned);
    details.body_param = body_param.map(str::to_owned);
    ErrorClass::BadRequest(Box::new(ResponseDetails::from(details)))
}

// BudgetExceededError reads a missing or empty message as absent and renders the costs themselves
fn budget(
    message: Option<&str>,
    current_cost: &str,
    max_budget: &str,
) -> Result<ErrorClass, OperationError> {
    Ok(ErrorClass::BudgetExceeded(Box::new(
        BudgetExceededDetails {
            current_cost: decimal(current_cost)?,
            max_budget: decimal(max_budget)?,
            message: message.map(str::to_owned),
            // BudgetExceededError's llm_provider defaults to the empty string
            provider: String::new(),
            entity_type: None,
            entity_id: None,
            status_code: 422,
        },
    )))
}

fn decimal(text: &str) -> Result<Decimal, OperationError> {
    Decimal::from_str(text).map_err(|source| OperationError::InvalidRequest {
        message: format!("{text} is not a decimal cost: {source}"),
    })
}

fn guardrail(message: &str, guardrail_name: Option<&str>) -> ErrorClass {
    let name = guardrail_name.unwrap_or("None");
    ErrorClass::GuardrailRaised(Box::new(GuardrailDetails {
        guardrail_name: guardrail_name.map(str::to_owned),
        message: format!("Guardrail raised an exception, Guardrail: {name}, Message: {message}"),
        status_code: 400,
        blocked_content: false,
    }))
}

fn json_schema(raw_response: &str, schema: &str) -> ErrorClass {
    ErrorClass::JsonSchemaValidation(Box::new(SchemaValidationDetails {
        model: MODEL.to_owned(),
        provider: PROVIDER.to_owned(),
        raw_response: raw_response.to_owned(),
        schema: schema.to_owned(),
    }))
}

fn header_found(case: &HeaderCase) -> HeaderFound {
    let response = attach_fallback_headers(
        response_for(case.kind, case.existing.as_ref()),
        case.attempted_fallbacks,
        case.errors.as_deref(),
    );
    let hidden = hidden_params(&response);
    let additional = ensure_additional_headers_dict(&hidden);
    let errors_present = additional.contains_key(FALLBACK_ERRORS_HEADER);
    let headers: JsonObject = additional
        .iter()
        .filter(|(name, _value)| name.as_str() != FALLBACK_ERRORS_HEADER)
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    HeaderFound {
        result: case.kind,
        has_hidden_params: has_hidden_params(&response),
        headers,
        errors_present,
        errors: fallback_errors_from_headers(&additional),
    }
}

fn response_for(kind: ResponseKind, existing: Option<&JsonObject>) -> Response {
    let mut hidden = JsonObject::new();
    if let Some(headers) = existing {
        hidden.insert(
            "additional_headers".to_owned(),
            Json::Object(headers.clone()),
        );
    }
    match kind {
        ResponseKind::Absent => Response::Absent,
        // a dict keeps its body and carries the hidden params under _hidden_params, which add_fallback_headers_to_response creates
        ResponseKind::Object => Response::Object(JsonObject::from_iter([(
            "_hidden_params".to_owned(),
            Json::Object(hidden),
        )])),
        ResponseKind::Attributed => Response::Attributed(hidden),
        ResponseKind::Opaque => Response::Opaque,
    }
}

fn hidden_params(response: &Response) -> JsonObject {
    match response {
        Response::Object(body) => match body.get("_hidden_params") {
            Some(Json::Object(hidden)) => hidden.clone(),
            None
            | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                JsonObject::new()
            }
        },
        Response::Attributed(hidden) => hidden.clone(),
        Response::Absent | Response::Stream | Response::Opaque => JsonObject::new(),
    }
}

fn has_hidden_params(response: &Response) -> bool {
    match response {
        Response::Object(body) => body.contains_key("_hidden_params"),
        Response::Attributed(_) | Response::Stream => true,
        Response::Absent | Response::Opaque => false,
    }
}

async fn events(cases: &[EventCase]) -> Vec<EventFound> {
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        found.push(event_found(case).await);
    }
    found
}

// log_success_fallback_event/log_failure_fallback_event hand every registered CustomLogger the same three values
async fn event_found(case: &EventCase) -> EventFound {
    let recording = Arc::new(Mutex::new(Vec::new()));
    let observers = observers(case, &recording);
    // log_success_fallback_event and log_failure_fallback_event pass a plain exception through to every observer
    let exception = ErrorClass::Unclassified(Box::new(context(case.message.clone())));
    log_fallback_event(
        &observers,
        outcome_of(case.outcome),
        FallbackEvent {
            original_model_group: &case.original_model_group,
            request: &case.request,
            exception: &exception,
        },
    )
    .await;
    let recorded = if let Ok(guard) = recording.lock() {
        guard.clone()
    } else {
        Vec::new()
    };
    EventFound { calls: recorded }
}

const fn outcome_of(outcome: Outcome) -> FallbackOutcome {
    match outcome {
        Outcome::Success => FallbackOutcome::Success,
        Outcome::Failure => FallbackOutcome::Failure,
    }
}

fn observers(
    case: &EventCase,
    calls: &Arc<Mutex<Vec<EventCall>>>,
) -> Vec<Arc<dyn FallbackObserver>> {
    case.observers
        .iter()
        .enumerate()
        .map(|(index, kind)| observer_for(*kind, u32::try_from(index).unwrap_or(u32::MAX), calls))
        .collect()
}

fn observer_for(
    kind: ObserverKind,
    index: u32,
    calls: &Arc<Mutex<Vec<EventCall>>>,
) -> Arc<dyn FallbackObserver> {
    match kind {
        ObserverKind::Record => Arc::new(RecordingObserver {
            index,
            calls: Arc::clone(calls),
        }),
        ObserverKind::Fail => Arc::new(FailingObserver),
        ObserverKind::Silent => Arc::new(SilentObserver),
    }
}

struct RecordingObserver {
    index: u32,
    calls: Arc<Mutex<Vec<EventCall>>>,
}

impl RecordingObserver {
    fn record(&self, outcome: Outcome, event: FallbackEvent<'_>) {
        let call = EventCall {
            observer: self.index,
            outcome,
            original_model_group: event.original_model_group.to_owned(),
            model: model_of(event.request),
            message: exception_message(event.exception),
        };
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }
}

fn model_of(request: &JsonObject) -> Option<String> {
    match request.get("model") {
        Some(Json::Text(model)) => Some(model.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

#[async_trait]
impl FallbackObserver for RecordingObserver {
    async fn log_success_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        self.record(Outcome::Success, event);
        Ok(())
    }

    async fn log_failure_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        self.record(Outcome::Failure, event);
        Ok(())
    }
}

struct FailingObserver;

fn observer_failure(event: &FallbackEvent<'_>) -> FallbackObservationError {
    FallbackObservationError::Observer {
        message: format!(
            "the observer failed for {} over {} values: {}",
            event.original_model_group,
            event.request.len(),
            exception_message(event.exception)
        ),
    }
}

#[async_trait]
impl FallbackObserver for FailingObserver {
    async fn log_success_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        Err(observer_failure(&event))
    }

    async fn log_failure_fallback_event(
        &self,
        event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        Err(observer_failure(&event))
    }
}

struct SilentObserver;

impl FallbackObserver for SilentObserver {}
