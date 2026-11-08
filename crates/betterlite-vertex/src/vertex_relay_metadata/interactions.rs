use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::interactions::record::{
    InteractionsUsage, is_interactions_usage_object,
};
use betterlite_models::cost_usage::interactions::transform::transform_interactions_usage;
use serde_json::value::RawValue;

use super::error::VertexRelayError;
use super::payload::{ResponseLoggingPayloadInput, response_logging_payload};
use super::types::{
    CompletionCost, CompletionResponse, CustomLlmProvider, LoggingObject, ResponseLoggingKwargs,
};

type ResponseBodyFields = BTreeMap<Box<str>, Box<RawValue>>;

pub struct InteractionsPassthroughInput<'input> {
    pub response_body: &'input [u8],
    pub request_model: Option<&'input str>,
    pub logging: &'input mut LoggingObject,
    pub kwargs: ResponseLoggingKwargs,
    pub custom_llm_provider: CustomLlmProvider,
    pub vertex_location: Option<&'input str>,
}

#[derive(Debug)]
pub struct InteractionsPassthroughOutcome {
    pub result: Option<CompletionResponse>,
    pub kwargs: ResponseLoggingKwargs,
}

pub fn interactions_handler(
    cost: &dyn CompletionCost,
    input: InteractionsPassthroughInput<'_>,
) -> Result<InteractionsPassthroughOutcome, VertexRelayError> {
    let body = parse_interactions_body(input.response_body)?;
    let Some((model, usage)) = project_interactions(&body, input.request_model)? else {
        return Ok(InteractionsPassthroughOutcome {
            result: None,
            kwargs: input.kwargs,
        });
    };
    let mut response = CompletionResponse {
        id: None,
        model: Some(model.clone()),
        usage: Some(usage),
    };
    input.logging.custom_llm_provider = Some(input.custom_llm_provider);
    let mut logged_kwargs = response_logging_payload(
        cost,
        ResponseLoggingPayloadInput {
            response: &mut response,
            model: &model,
            kwargs: input.kwargs,
            logging: input.logging,
            custom_llm_provider: input.custom_llm_provider,
            vertex_location: input.vertex_location,
        },
    );
    logged_kwargs.custom_llm_provider = Some(input.custom_llm_provider);
    Ok(InteractionsPassthroughOutcome {
        result: Some(response),
        kwargs: logged_kwargs,
    })
}

fn parse_interactions_body(response_body: &[u8]) -> Result<ResponseBodyFields, VertexRelayError> {
    serde_json::from_slice(response_body).map_err(|source| VertexRelayError::ResponseBody {
        source: Box::new(source),
    })
}

fn project_interactions(
    fields: &ResponseBodyFields,
    request_model: Option<&str>,
) -> Result<Option<(String, CostUsage)>, VertexRelayError> {
    let Some(model) = interactions_model(fields, request_model) else {
        return Ok(None);
    };
    let Some(record) = interactions_usage(fields) else {
        return Ok(None);
    };
    let usage =
        transform_interactions_usage(&record).map_err(|source| VertexRelayError::Usage {
            source: Box::new(source),
        })?;
    Ok(Some((model, usage)))
}

fn interactions_model(fields: &ResponseBodyFields, request_model: Option<&str>) -> Option<String> {
    let response_model = fields
        .get("model")
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
        .filter(|model| !model.is_empty());
    if let Some(model) = response_model {
        return Some(model);
    }
    request_model
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

// A derived struct also accepts an array by position, while the Python contract requires an object.
fn interactions_usage(fields: &ResponseBodyFields) -> Option<InteractionsUsage> {
    let raw = fields.get("usage")?;
    if !raw.get().trim_start().starts_with('{') {
        return None;
    }
    let usage: InteractionsUsage = serde_json::from_str(raw.get()).ok()?;
    is_interactions_usage_object(&usage).then_some(usage)
}
