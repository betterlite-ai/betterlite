use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::interactions::record::{
    InteractionsUsage, is_interactions_usage_object,
};
use betterlite_models::cost_usage::interactions::transform::transform_interactions_usage;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::value::RawValue;

use super::error::VertexRelayError;
use super::payload::{ResponseLoggingPayloadInput, response_logging_payload};
use super::types::{
    CompletionCost, CompletionResponse, CustomLlmProvider, LoggingObject, ResponseLoggingKwargs,
};

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

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct InteractionsResponseBody {
    model: ModelName,
    usage: Option<Box<RawValue>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(from = "RawModelName")]
struct ModelName(Option<String>);

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawModelName {
    Str(Box<str>),
    Other(Box<IgnoredAny>),
}

impl From<RawModelName> for ModelName {
    fn from(raw: RawModelName) -> Self {
        match raw {
            RawModelName::Str(model) => Self(Some(String::from(model))),
            RawModelName::Other(_ignored) => Self(None),
        }
    }
}

impl InteractionsResponseBody {
    fn interactions_model(&self, request_model: Option<&str>) -> Option<String> {
        if let Some(model) = self.model.0.as_deref().filter(|model| !model.is_empty()) {
            return Some(model.to_owned());
        }
        request_model
            .filter(|model| !model.is_empty())
            .map(str::to_owned)
    }

    fn interactions_usage(&self) -> Option<InteractionsUsage> {
        let raw = self.usage.as_deref()?;
        let usage: InteractionsUsage = serde_json::from_str(raw.get()).ok()?;
        is_interactions_usage_object(&usage).then_some(usage)
    }
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

fn parse_interactions_body(
    response_body: &[u8],
) -> Result<InteractionsResponseBody, VertexRelayError> {
    serde_json::from_slice(response_body).map_err(|source| VertexRelayError::ResponseBody {
        source: Box::new(source),
    })
}

fn project_interactions(
    body: &InteractionsResponseBody,
    request_model: Option<&str>,
) -> Result<Option<(String, CostUsage)>, VertexRelayError> {
    let Some(model) = body.interactions_model(request_model) else {
        return Ok(None);
    };
    let Some(record) = body.interactions_usage() else {
        return Ok(None);
    };
    let usage =
        transform_interactions_usage(&record).map_err(|source| VertexRelayError::Usage {
            source: Box::new(source),
        })?;
    Ok(Some((model, usage)))
}
