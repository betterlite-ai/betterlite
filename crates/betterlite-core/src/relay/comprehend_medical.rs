use std::sync::Arc;

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::api::media_relay_logging::spend::decimal_json;

mod tests;

const PROVIDER: &str = "comprehendmedical";
const CHARS_PER_UNIT: u64 = 100;

pub struct ComprehendMedicalInput<'call> {
    pub logging_obj: &'call StreamLoggingObject,
    pub request_headers: &'call HeaderMap,
    pub request_body: &'call Json,
    pub url_route: &'call str,
    pub result: &'call str,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub kwargs: PassThroughLoggingKwargs,
}

#[derive(Debug, Error)]
enum ComprehendMedicalError {
    #[error("the Comprehend Medical request body must be a JSON object, found {found}")]
    RequestBodyShape { found: &'static str },
}

pub struct ComprehendMedicalLogger {
    payload_builder: Arc<dyn StandardPayloadBuilder>,
}

impl ComprehendMedicalLogger {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - B3606 constructs the logger through this fixed name
    pub fn new(payload_builder: Arc<dyn StandardPayloadBuilder>) -> Self {
        Self { payload_builder }
    }

    // LiteLLM wraps the body in try/except: the failure logs its error and returns the relayed result with the caller's kwargs.
    #[must_use]
    pub fn comprehend_medical_handler(&self, input: ComprehendMedicalInput<'_>) -> HandlerOutcome {
        match self.run(&input) {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::error!("Error in Comprehend Medical passthrough logging handler: {error}");
                HandlerOutcome {
                    result: Some(standard_result(input.result)),
                    kwargs: input.kwargs,
                }
            }
        }
    }

    fn run(
        &self,
        input: &ComprehendMedicalInput<'_>,
    ) -> Result<HandlerOutcome, ComprehendMedicalError> {
        let text = request_text(input.request_body)?;
        let operation = operation_from_headers(input.request_headers);
        let response_cost = cost_for_operation(&operation, text);
        let model_name = format!("comprehendmedical/{operation}");
        let mut kwargs = record(input, &model_name, response_cost);
        let init_response = response_object(input.result);
        let payload = self.build_payload(input, &init_response);
        kwargs.entries.insert(
            STANDARD_LOGGING_OBJECT.to_owned(),
            payload_entry(payload.as_ref()),
        );
        Ok(HandlerOutcome {
            result: Some(standard_result(input.result)),
            kwargs,
        })
    }

    fn build_payload(
        &self,
        input: &ComprehendMedicalInput<'_>,
        init_response: &Json,
    ) -> Option<StandardLoggingPayload> {
        input
            .logging_obj
            .read(&mut |state| {
                self.payload_builder
                    .build(state, init_response, input.start_time, input.end_time)
            })
            .flatten()
    }
}

// LiteLLM writes the model, provider and cost into both the returned kwargs and the call details.
fn record(
    input: &ComprehendMedicalInput<'_>,
    model_name: &str,
    response_cost: Decimal,
) -> PassThroughLoggingKwargs {
    let details = IndexMap::from([
        ("model".to_owned(), Json::Text(model_name.to_owned())),
        (
            "custom_llm_provider".to_owned(),
            Json::Text(PROVIDER.to_owned()),
        ),
        ("response_cost".to_owned(), decimal_json(response_cost)),
    ]);
    let mut kwargs = input.kwargs.clone();
    kwargs.entries.extend(details.clone());
    input.logging_obj.edit(&mut |state| {
        model_name.clone_into(&mut state.model);
        state.custom_llm_provider = Some(PROVIDER.to_owned());
        for (key, value) in &details {
            state.model_call_details.insert(key.clone(), value.clone());
        }
    });
    kwargs
}

// The tariff table is the operation list B3621 checks against; the tariffs are USD per started 100-character unit.
pub fn supported_operations() -> impl Iterator<Item = &'static str> {
    tariff()
        .into_iter()
        .map(|(operation, _unit_cost)| operation)
}

// LiteLLM's COMPREHEND_MEDICAL_COST_PER_UNIT_USD as (mantissa, scale): 0.01, 0.0014, 0.0005, 0.00025, 0.0075.
fn tariff() -> [(&'static str, Decimal); 5] {
    [
        ("DetectEntitiesV2", Decimal::new(1, 2)),
        ("DetectPHI", Decimal::new(14, 4)),
        ("InferICD10CM", Decimal::new(5, 4)),
        ("InferRxNorm", Decimal::new(25, 5)),
        ("InferSNOMEDCT", Decimal::new(75, 4)),
    ]
}

fn unit_cost(operation: &str) -> Option<Decimal> {
    tariff()
        .into_iter()
        .find(|(name, _unit_cost)| *name == operation)
        .map(|(_name, unit_cost)| unit_cost)
}

// x-amz-target reads "ComprehendMedical_V20181030.DetectPHI", and its last dot-separated part names the operation.
fn operation_from_headers(headers: &HeaderMap) -> String {
    let target = headers
        .get("x-amz-target")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    target
        .rsplit_once('.')
        .map_or(target, |(_head, tail)| tail)
        .to_owned()
}

// Python reads a non-string Text as ""; a request body that is not an object raises there, which its except logs and returns.
fn request_text(body: &Json) -> Result<&str, ComprehendMedicalError> {
    let Json::Object(fields) = body else {
        return Err(ComprehendMedicalError::RequestBodyShape {
            found: json_kind(body),
        });
    };
    let Some(Json::Text(text)) = fields.get("Text") else {
        return Ok("");
    };
    Ok(text.as_str())
}

const fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::Text(_) => "a string",
        Json::List(_) => "a list",
        Json::Object(_) => "an object",
    }
}

// math.ceil(len(text) / 100) with a 1-unit minimum, charged at the operation's tariff; an unknown operation costs nothing.
fn cost_for_operation(operation: &str, text: &str) -> Decimal {
    let Some(unit_cost) = unit_cost(operation) else {
        return Decimal::ZERO;
    };
    let characters = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
    let units = Decimal::from(characters.div_ceil(CHARS_PER_UNIT)).max(Decimal::ONE);
    units.saturating_mul(unit_cost)
}

fn standard_result(result: &str) -> PassThroughResponse {
    PassThroughResponse::Standard(PassThroughResponseObject {
        response: Json::Text(result.to_owned()),
    })
}

fn response_object(result: &str) -> Json {
    Json::Object(IndexMap::from([(
        "response".to_owned(),
        Json::Text(result.to_owned()),
    )]))
}

// A payload that cannot be written as JSON stands in as Null, the way a failed build does in LiteLLM.
fn payload_entry(payload: Option<&StandardLoggingPayload>) -> Json {
    payload
        .and_then(|built| serde_json::to_string(built).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Json::Null)
}
