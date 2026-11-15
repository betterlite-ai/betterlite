use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_anthropic::usage::{
    AnthropicUsage, ReasoningTokenCounter, calculate_usage as calculate_anthropic_usage,
};
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{
    CompletionTokensDetails, PromptTokensDetails, Usage as SummedUsage,
};
use indexmap::IndexMap;
use serde_json::Value;

use super::BatchOutputDiagnostic;
use super::BatchOutputProvider;
use super::decode::{empty_object, object_value, serde_object};
use super::error::BatchLineError;
use crate::api::responses_usage::{
    Usage as ResponsesUsage, is_response_api_usage, transform_response_api_usage,
};
use crate::cost::completion::usage::pricing::pricing_usage;

pub(super) struct LineUsage {
    pub usage: SummedUsage,
    pub unparsed: Option<BatchOutputDiagnostic>,
}

struct NoReasoningCounter;

impl ReasoningTokenCounter for NoReasoningCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

pub(super) fn response_was_successful(entry: &Json, provider: BatchOutputProvider) -> bool {
    let Json::Object(fields) = entry else {
        return false;
    };
    match provider {
        BatchOutputProvider::Anthropic => match fields.get("result") {
            Some(Json::Object(result)) => succeeded_type(result.get("type")),
            _ => false,
        },
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

fn is_ok_status(status: Option<&Json>) -> bool {
    match status {
        Some(Json::Number(number)) => number.as_f64() == Some(200.0),
        _ => false,
    }
}

fn succeeded_type(kind: Option<&Json>) -> bool {
    match kind {
        Some(Json::Text(kind)) => kind == "succeeded",
        _ => false,
    }
}

pub(super) fn response_body<'line>(
    entry: &'line Json,
    provider: BatchOutputProvider,
) -> Result<Option<&'line IndexMap<String, Json>>, BatchLineError> {
    let Json::Object(fields) = entry else {
        return Ok(None);
    };
    match provider {
        BatchOutputProvider::Anthropic => {
            let Some(result) = super::decode::object_field(fields, "result")? else {
                return Ok(None);
            };
            super::decode::object_field(result, "message")
        }
        BatchOutputProvider::Bedrock => super::decode::object_field(fields, "modelOutput"),
        BatchOutputProvider::Openai
        | BatchOutputProvider::Azure
        | BatchOutputProvider::VertexAi
        | BatchOutputProvider::HostedVllm
        | BatchOutputProvider::Mistral => {
            let Some(response) = super::decode::object_field(fields, "response")? else {
                return Ok(None);
            };
            super::decode::object_field(response, "body")
        }
    }
}

pub(super) fn response_model(body: Option<&IndexMap<String, Json>>) -> Option<String> {
    match body?.get("model") {
        Some(Json::Text(model)) if !model.is_empty() => Some(model.clone()),
        _ => None,
    }
}

pub(super) fn usage_for_line(
    body: Option<&IndexMap<String, Json>>,
    provider: BatchOutputProvider,
    providers: &dyn super::BatchOutputProviders,
) -> Result<LineUsage, BatchLineError> {
    let Some(body) = body else {
        return Ok(empty_line_usage());
    };
    if provider.is_anthropic_family() {
        return anthropic_family_usage(body, provider, providers);
    }
    let Some(usage_value) = object_value(body, "usage")? else {
        return Ok(empty_line_usage());
    };
    if empty_object(usage_value) {
        return Ok(empty_line_usage());
    }
    Ok(LineUsage {
        usage: generic_usage(usage_value)?,
        unparsed: None,
    })
}

fn empty_line_usage() -> LineUsage {
    LineUsage {
        usage: SummedUsage::default(),
        unparsed: None,
    }
}

fn anthropic_family_usage(
    body: &IndexMap<String, Json>,
    provider: BatchOutputProvider,
    providers: &dyn super::BatchOutputProviders,
) -> Result<LineUsage, BatchLineError> {
    if provider == BatchOutputProvider::Bedrock
        && let Some(usage) = providers.bedrock_embedding_usage(body)
    {
        return Ok(LineUsage {
            usage,
            unparsed: None,
        });
    }
    let Some(usage_value) = object_value(body, "usage")? else {
        return Ok(empty_line_usage());
    };
    if empty_object(usage_value) {
        return Ok(empty_line_usage());
    }
    if provider == BatchOutputProvider::Bedrock {
        match providers.bedrock_converse_usage(usage_value) {
            super::ConverseUsageOutcome::Usage(usage) => {
                return Ok(LineUsage {
                    usage,
                    unparsed: None,
                });
            }
            super::ConverseUsageOutcome::Invalid => {
                return Err(BatchLineError::ConverseUsage);
            }
            super::ConverseUsageOutcome::NotConverse => {}
        }
    }
    anthropic_shape_usage(usage_value, provider)
}

fn anthropic_shape_usage(
    usage_value: &Json,
    provider: BatchOutputProvider,
) -> Result<LineUsage, BatchLineError> {
    let converted = calculate_anthropic_usage(usage_value, None, None, None, &NoReasoningCounter)?;
    let usage = from_anthropic_usage(&converted);
    if usage.total_tokens != 0 {
        return Ok(LineUsage {
            usage,
            unparsed: None,
        });
    }
    let usage_keys = sorted_keys(usage_value);
    tracing::warn!(
        provider = provider.slug(),
        usage_keys = ?usage_keys,
        "batch output line reported usage this parser does not understand, so it will be billed at $0"
    );
    Ok(LineUsage {
        usage,
        unparsed: Some(BatchOutputDiagnostic::UnparsedUsage {
            provider: provider.slug(),
            usage_keys,
        }),
    })
}

fn sorted_keys(value: &Json) -> Vec<String> {
    let Json::Object(fields) = value else {
        return Vec::new();
    };
    let mut keys: Vec<String> = fields.keys().cloned().collect();
    keys.sort();
    keys
}

fn generic_usage(value: &Json) -> Result<SummedUsage, BatchLineError> {
    let Json::Object(fields) = value else {
        return Err(BatchLineError::NotAnObject { field: "usage" });
    };
    let object = serde_object(fields)?;
    if is_response_api_usage(&object) {
        let value = Value::Object(object);
        let converted = transform_response_api_usage(Some(&value))?;
        return Ok(from_responses_usage(&converted));
    }
    let logging = usage_from_mapping(fields)?;
    Ok(pricing_usage(&logging)?)
}

fn from_responses_usage(usage: &ResponsesUsage) -> SummedUsage {
    SummedUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: usage.prompt_tokens_details.clone(),
        completion_tokens_details: usage.completion_tokens_details.clone(),
        numeric_extras: BTreeMap::new(),
    }
}

fn from_anthropic_usage(usage: &AnthropicUsage) -> SummedUsage {
    SummedUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: Some(PromptTokensDetails {
            text_tokens: Some(usage.prompt_tokens_details.text),
            audio_tokens: usage.prompt_tokens_details.audio,
            image_tokens: usage.prompt_tokens_details.image,
            video_tokens: usage.prompt_tokens_details.video,
            web_search_requests: None,
            google_maps_grounding_requests: None,
            tool_use_tokens: None,
            character_count: None,
            image_count: None,
            video_length_seconds: None,
            audio_length_seconds: None,
            query_count: None,
            cache_write_tokens: Some(usage.prompt_tokens_details.cache_write),
            cache_creation_tokens: Some(usage.prompt_tokens_details.cache_creation),
            cached_tokens: Some(usage.prompt_tokens_details.cached),
            cached_tokens_details: None,
        }),
        completion_tokens_details: Some(CompletionTokensDetails {
            text_tokens: usage.completion_tokens_details.text,
            audio_tokens: usage.completion_tokens_details.audio,
            image_tokens: usage.completion_tokens_details.image,
            video_tokens: usage.completion_tokens_details.video,
            reasoning_tokens: usage.completion_tokens_details.reasoning,
            accepted_prediction_tokens: usage.completion_tokens_details.accepted_prediction,
            rejected_prediction_tokens: usage.completion_tokens_details.rejected_prediction,
        }),
        numeric_extras: BTreeMap::new(),
    }
}
