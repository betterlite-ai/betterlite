use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::api::agents::merge_agent_headers;
use thiserror::Error;

use super::completion::CompletionCall;
use super::wire::{CompletionParams, MessageSendParams};
use super::{A2A_USER_API_KEY_HASH_PARAM, apply_forward_metadata, to_openai_messages};
use crate::client::records::AGENT_CARD_PATH_PARAM;

mod tests;

const AGENT_ONLY_PARAMS: [&str; 6] = [
    "is_public",
    "agent_name",
    "agent_id",
    "agent_card_params",
    AGENT_CARD_PATH_PARAM,
    A2A_USER_API_KEY_HASH_PARAM,
];

#[derive(Debug, Error)]
pub enum ParamsError {
    #[error("the A2A litellm_params field '{field}' must be a string")]
    NotText { field: &'static str },
    #[error("the A2A litellm_params field '{field}' must be a list of OpenAI messages")]
    NotMessages { field: &'static str },
    #[error("the A2A litellm_params field '{field}' must be a map of string headers")]
    NotHeaders {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

pub struct BridgeCall<'call> {
    pub request_id: &'call str,
    pub params: &'call MessageSendParams,
    pub litellm_params: &'call BTreeMap<String, AgentJson>,
    pub api_base: Option<&'call str>,
    pub agent_extra_headers: Option<&'call BTreeMap<String, String>>,
}

pub fn build_completion_params(
    call: &BridgeCall<'_>,
    stream: bool,
) -> Result<CompletionCall, ParamsError> {
    let model = prefixed_model(call)?;
    let messages = forwarded_messages(call)?;
    let mut extra = copied_params(call);
    if let Some(api_base) = call.api_base {
        extra
            .entry("api_base".to_owned())
            .or_insert_with(|| AgentJson::String(api_base.to_owned()));
    }
    layered_params(&mut extra, call);
    if let Some(headers) = call
        .agent_extra_headers
        .filter(|headers| !headers.is_empty())
    {
        merged_headers(&mut extra, headers)?;
    }
    Ok(CompletionCall {
        model,
        messages,
        stream,
        extra,
    })
}

pub(crate) fn optional_text<'map>(
    map: &'map BTreeMap<String, AgentJson>,
    field: &'static str,
) -> Result<Option<&'map str>, ParamsError> {
    let Some(value) = map.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(Some)
        .ok_or(ParamsError::NotText { field })
}

fn prefixed_model(call: &BridgeCall<'_>) -> Result<String, ParamsError> {
    let custom_llm_provider = optional_text(call.litellm_params, "custom_llm_provider")?;
    let model = optional_text(call.litellm_params, "model")?.unwrap_or("agent");
    match custom_llm_provider.filter(|provider| !provider.is_empty()) {
        Some(provider) if !model.starts_with(&format!("{provider}/")) => {
            Ok(format!("{provider}/{model}"))
        }
        _ => Ok(model.to_owned()),
    }
}

fn forwarded_messages(call: &BridgeCall<'_>) -> Result<Vec<AgentJson>, ParamsError> {
    match call.litellm_params.get("messages") {
        None | Some(AgentJson::Null) => Ok(to_openai_messages(&call.params.message)),
        Some(AgentJson::Array(messages)) => Ok(messages.clone()),
        Some(_) => Err(ParamsError::NotMessages { field: "messages" }),
    }
}

fn copied_params(call: &BridgeCall<'_>) -> BTreeMap<String, AgentJson> {
    call.litellm_params
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "model" | "custom_llm_provider" | "messages" | "stream"
            ) && !AGENT_ONLY_PARAMS.contains(&name.as_str())
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn layered_params(extra: &mut BTreeMap<String, AgentJson>, call: &BridgeCall<'_>) {
    let mut forwarded = CompletionParams {
        extra_body: extra.remove("extra_body"),
        metadata: None,
        extra: BTreeMap::new(),
    };
    apply_forward_metadata(&mut forwarded, &call.params.message, Some(call.params));
    if let Some(extra_body) = forwarded.extra_body {
        extra.insert("extra_body".to_owned(), extra_body);
    }
}

fn merged_headers(
    extra: &mut BTreeMap<String, AgentJson>,
    dynamic: &BTreeMap<String, String>,
) -> Result<(), ParamsError> {
    let static_headers = match extra.get("extra_headers") {
        None | Some(AgentJson::Null) => None,
        Some(value) => Some(
            serde_json::from_value::<BTreeMap<String, String>>(value.clone()).map_err(
                |source| ParamsError::NotHeaders {
                    field: "extra_headers",
                    source,
                },
            )?,
        ),
    };
    if let Some(merged) = merge_agent_headers(Some(dynamic), static_headers.as_ref()) {
        let headers = merged
            .into_iter()
            .map(|(name, value)| (name, AgentJson::String(value)))
            .collect();
        extra.insert("extra_headers".to_owned(), AgentJson::Object(headers));
    }
    Ok(())
}
