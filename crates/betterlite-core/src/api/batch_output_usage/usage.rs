use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_anthropic::usage::{
    ReasoningTokenCounter, calculate_usage as calculate_anthropic_usage,
};
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage as SummedUsage;
use indexmap::IndexMap;

use super::decode::object_field;
use super::error::{BatchOutputUsageError, BatchResult};
use super::{BatchOutputProvider, BatchOutputProviders, ConverseUsageOutcome};
use crate::api::responses_usage::{is_response_api_usage, transform_response_api_usage};
use crate::cost::completion::usage::pricing::pricing_usage;

struct NoReasoningCounter;

impl ReasoningTokenCounter for NoReasoningCounter {
    type Failure = Infallible;

    // The batch path always passes reasoning_content None, so no value reaches this body; a mutant of it never runs.
    #[cfg_attr(test, mutants::skip)]
    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

fn succeeded(kind: Option<&Json>) -> bool {
    let Some(Json::Text(found)) = kind else {
        return false;
    };
    found.as_str() == "succeeded"
}

// LiteLLM compares the line's status code with 200; comparing the bits keeps that exact.
const OK_STATUS: f64 = 200.0;

fn is_ok_status(status: Option<&Json>) -> bool {
    let Some(Json::Number(number)) = status else {
        return false;
    };
    number
        .as_f64()
        .is_some_and(|value| value.to_bits() == OK_STATUS.to_bits())
}

pub(super) fn response_was_successful(entry: &Json, provider: BatchOutputProvider) -> bool {
    let Json::Object(fields) = entry else {
        return false;
    };
    match provider {
        BatchOutputProvider::Anthropic => {
            matches!(fields.get("result"), Some(Json::Object(result)) if succeeded(result.get("type")))
        }
        BatchOutputProvider::Bedrock => {
            !matches!(fields.get("modelOutput"), None | Some(Json::Null))
                && matches!(fields.get("error"), None | Some(Json::Null))
        }
        BatchOutputProvider::Openai
        | BatchOutputProvider::Azure
        | BatchOutputProvider::VertexAi
        | BatchOutputProvider::HostedVllm
        | BatchOutputProvider::Mistral => {
            matches!(fields.get("response"), Some(Json::Object(response)) if is_ok_status(response.get("status_code")))
        }
    }
}

pub(super) fn response_body(
    entry: &Json,
    provider: BatchOutputProvider,
) -> BatchResult<Option<&IndexMap<String, Json>>> {
    let Json::Object(fields) = entry else {
        return Ok(None);
    };
    if provider == BatchOutputProvider::Bedrock {
        return object_field(fields, "modelOutput");
    }
    let (outer, inner) = if provider == BatchOutputProvider::Anthropic {
        ("result", "message")
    } else {
        ("response", "body")
    };
    let Some(found) = object_field(fields, outer)? else {
        return Ok(None);
    };
    object_field(found, inner)
}

pub(super) fn response_model(body: Option<&IndexMap<String, Json>>) -> Option<String> {
    let Some(Json::Text(model)) = body?.get("model") else {
        return None;
    };
    if model.is_empty() {
        return None;
    }
    Some(model.clone())
}

pub(super) fn usage_for_line(
    body: Option<&IndexMap<String, Json>>,
    provider: BatchOutputProvider,
    providers: &dyn BatchOutputProviders,
) -> BatchResult<SummedUsage> {
    let Some(body_fields) = body else {
        return Ok(SummedUsage::default());
    };
    if provider == BatchOutputProvider::Bedrock
        && let Some(usage) = providers.bedrock_embedding_usage(body_fields)
    {
        return Ok(usage);
    }
    let Some(usage_fields) = object_field(body_fields, "usage")? else {
        return Ok(SummedUsage::default());
    };
    if usage_fields.is_empty() {
        return Ok(SummedUsage::default());
    }
    let usage_document = Json::Object(usage_fields.clone());
    if provider == BatchOutputProvider::Bedrock {
        return match providers.bedrock_converse_usage(&usage_document) {
            ConverseUsageOutcome::Usage(usage) => Ok(*usage),
            ConverseUsageOutcome::Invalid => Err(BatchOutputUsageError::ConverseUsage),
            ConverseUsageOutcome::NotConverse => anthropic_usage(&usage_document, provider),
        };
    }
    if provider == BatchOutputProvider::Anthropic {
        return anthropic_usage(&usage_document, provider);
    }
    generic_usage(usage_fields)
}

// The cost path prices a usage mapping, so the converted Anthropic usage rejoins that parser.
fn anthropic_usage(value: &Json, provider: BatchOutputProvider) -> BatchResult<SummedUsage> {
    let converted = calculate_anthropic_usage(value, None, None, None, &NoReasoningCounter)?;
    let text = serde_json::to_string(&converted)?;
    let fields: IndexMap<String, Json> = serde_json::from_str(&text)?;
    let usage = pricing_usage(&usage_from_mapping(&fields)?)?;
    if usage.total_tokens == 0 {
        tracing::warn!(
            provider = provider.slug(),
            "batch output line reported usage this parser does not understand, so it will be billed at $0"
        );
    }
    Ok(usage)
}

fn generic_usage(fields: &IndexMap<String, Json>) -> BatchResult<SummedUsage> {
    let document = serde_json::to_value(fields)?;
    if let Some(object) = document.as_object()
        && is_response_api_usage(object)
    {
        let converted = transform_response_api_usage(Some(&document))?;
        return Ok(SummedUsage {
            prompt_tokens: converted.prompt_tokens,
            completion_tokens: converted.completion_tokens,
            total_tokens: converted.total_tokens,
            prompt_tokens_details: converted.prompt_tokens_details,
            completion_tokens_details: converted.completion_tokens_details,
            numeric_extras: BTreeMap::new(),
        });
    }
    Ok(pricing_usage(&usage_from_mapping(fields)?)?)
}
