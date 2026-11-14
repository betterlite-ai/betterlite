use std::collections::BTreeMap;

use betterlite_models::cost_usage::{CompletionDetails, CostUsage};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

use crate::chat::choice_finish_reason;
use crate::chat::response::{ChatResponse, ChatUsage};
use crate::xai_chat::error::XaiChatError;
use crate::xai_usage::error::XaiUsageError;
use crate::xai_usage::reconcile::{
    fold_reasoning, normalize_totals, usage_restated_from_xai_ticks,
};
use crate::xai_usage::{XaiUsage, attach_tool_details};

pub struct ReplyInput<'request> {
    pub body: &'request [u8],
}

pub struct Reply {
    pub response: ChatResponse,
    pub usage: XaiUsage,
}

// the usage block as xAI sends it; cost_in_usd_ticks stays untyped so a boolean is told apart from an integer
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireUsage {
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) total_tokens: i64,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub(crate) cost: Option<Decimal>,
    pub(crate) cost_in_usd_ticks: Option<Value>,
    pub(crate) server_side_tool_usage_details: Option<BTreeMap<String, Value>>,
    pub(crate) completion_tokens_details: Option<WireCompletionDetails>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireCompletionDetails {
    pub(crate) reasoning_tokens: Option<i64>,
}

impl WireUsage {
    pub(crate) fn into_xai(self) -> XaiUsage {
        XaiUsage {
            tokens: CostUsage {
                prompt_tokens: self.prompt_tokens,
                completion_tokens: self.completion_tokens,
                total_tokens: self.total_tokens,
                prompt_details: None,
                completion_details: self.completion_tokens_details.map(|details| {
                    CompletionDetails {
                        text_tokens: None,
                        audio_tokens: None,
                        image_tokens: None,
                        video_tokens: None,
                        reasoning_tokens: details.reasoning_tokens,
                    }
                }),
                cache_read_tokens: None,
                numeric_extras: BTreeMap::new(),
            },
            cost: self.cost,
            cost_in_usd_ticks: self.cost_in_usd_ticks,
            server_side_tool_usage_details: self.server_side_tool_usage_details,
        }
    }
}

// XAIChatConfig.transform_response: the parent reply, the finish repair, then B1402's three usage steps
pub fn transform_response(input: &ReplyInput<'_>) -> Result<Reply, XaiChatError> {
    let mut raw: Value =
        serde_json::from_slice(input.body).map_err(|source| XaiChatError::Decode { source })?;
    // LiteLLM reads usage only from a non-null object, so a null block counts as none
    if let Some(fields) = raw.as_object_mut()
        && fields.get("usage").is_some_and(Value::is_null)
    {
        fields.remove("usage");
    }
    let mut response: ChatResponse =
        serde_json::from_value(raw.clone()).map_err(|source| XaiChatError::Decode { source })?;
    // the parent's Choices maps "" to "stop" before XAIChatConfig tests for "", so that repair never fires
    for choice in &mut response.choices {
        let calls = choice
            .message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty());
        let finish_reason = choice.finish_reason.as_deref();
        let (mapped, native) = choice_finish_reason(finish_reason);
        // an absent or null finish_reason defaults to stop before the tool_calls rewrite
        choice.finish_reason = Some(
            if calls && finish_reason.is_none_or(|reason| reason == "stop") {
                String::from("tool_calls")
            } else {
                mapped
            },
        );
        choice.provider_specific_fields = native;
    }
    let mut usage = usage_from_parent(&response.usage)?;
    if let Some(fields) = raw.get("usage").and_then(Value::as_object) {
        match serde_json::from_value::<WireUsage>(Value::Object(fields.clone())) {
            Ok(wire) => {
                usage.cost = wire.cost;
                usage.cost_in_usd_ticks = wire.cost_in_usd_ticks;
                attach_tool_details(&mut usage, wire.server_side_tool_usage_details);
            }
            Err(error) => {
                tracing::debug!(%error, "Error extracting X.AI server_side_tool_usage_details");
            }
        }
    }
    fold_reasoning(&mut usage)?;
    normalize_totals(&mut usage)?;
    if let Some(restated) = usage_restated_from_xai_ticks(&usage) {
        usage = restated;
    }
    write_back(&mut response.usage, &usage)?;
    Ok(Reply { response, usage })
}

fn usage_from_parent(usage: &ChatUsage) -> Result<XaiUsage, XaiChatError> {
    let reasoning = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| details.get("reasoning_tokens"))
        .copied()
        .flatten()
        .map(i64::try_from)
        .transpose()
        .map_err(|_| XaiChatError::Usage(XaiUsageError::Overflow))?;
    Ok(XaiUsage {
        tokens: CostUsage {
            prompt_tokens: i64::try_from(usage.prompt_tokens)
                .map_err(|_| XaiChatError::Usage(XaiUsageError::Overflow))?,
            completion_tokens: i64::try_from(usage.completion_tokens)
                .map_err(|_| XaiChatError::Usage(XaiUsageError::Overflow))?,
            total_tokens: i64::try_from(usage.total_tokens)
                .map_err(|_| XaiChatError::Usage(XaiUsageError::Overflow))?,
            prompt_details: None,
            completion_details: reasoning.map(|tokens| CompletionDetails {
                text_tokens: None,
                audio_tokens: None,
                image_tokens: None,
                video_tokens: None,
                reasoning_tokens: Some(tokens),
            }),
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        },
        cost: None,
        cost_in_usd_ticks: None,
        server_side_tool_usage_details: None,
    })
}

fn write_back(usage: &mut ChatUsage, reconciled: &XaiUsage) -> Result<(), XaiChatError> {
    usage.prompt_tokens = u64::try_from(reconciled.tokens.prompt_tokens)
        .map_err(|_| XaiChatError::Usage(XaiUsageError::Overflow))?;
    usage.completion_tokens = u64::try_from(reconciled.tokens.completion_tokens)
        .map_err(|_| XaiChatError::Usage(XaiUsageError::Overflow))?;
    usage.total_tokens = u64::try_from(reconciled.tokens.total_tokens)
        .map_err(|_| XaiChatError::Usage(XaiUsageError::Overflow))?;
    if let Some(requests) = reconciled
        .tokens
        .prompt_details
        .and_then(|details| details.web_search_requests)
    {
        let requests =
            u64::try_from(requests).map_err(|_| XaiChatError::Usage(XaiUsageError::Overflow))?;
        usage
            .prompt_tokens_details
            .get_or_insert_default()
            .insert(String::from("web_search_requests"), Some(requests));
    }
    Ok(())
}
