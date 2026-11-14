use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::compresr_transport::api_base::validate_api_base;
use betterlite_guardrails::compresr_transport::content::replace_text_in_content;
use betterlite_guardrails::compresr_transport::query::{
    fallback_query, query_for_target, render_tool_intent,
};
use betterlite_guardrails::compresr_transport::response::{safe_int, safe_response_text};
use betterlite_guardrails::compresr_transport::{DEFAULT_COMPRESS_TIMEOUT, TransportSettings};
use betterlite_guardrails::generic_guardrail_contracts::{GuardrailMessage, UnreachableFallback};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod compress;
mod logs;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    api_base: String,
    model: String,
    target_compression_ratio: f64,
    coarse: bool,
    dynamic: bool,
    dynamic_min_ratio: Option<f64>,
    dynamic_max_ratio: Option<f64>,
    compression_params: BTreeMap<String, GuardrailValue>,
    timeout: Option<f64>,
    fallback: UnreachableFallback,
}

impl Settings {
    fn transport(&self) -> TransportSettings {
        TransportSettings {
            api_base: self.api_base.clone(),
            model: self.model.clone(),
            target_compression_ratio: self.target_compression_ratio,
            coarse: self.coarse,
            dynamic: self.dynamic,
            dynamic_min_ratio: self.dynamic_min_ratio,
            dynamic_max_ratio: self.dynamic_max_ratio,
            compression_params: self.compression_params.clone(),
            timeout: self
                .timeout
                .map_or(DEFAULT_COMPRESS_TIMEOUT, Duration::from_secs_f64),
            fallback: self.fallback,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ValidateApiBase {
        settings: Settings,
    },
    QueryForTarget {
        messages: Vec<GuardrailMessage>,
        target_idx: usize,
        fallback: String,
    },
    RenderToolIntent {
        function_call: GuardrailValue,
    },
    FallbackQuery {
        messages: Vec<GuardrailMessage>,
    },
    ReplaceTextInContent {
        content: GuardrailValue,
        new_text: String,
    },
    SafeInt {
        value: GuardrailValue,
    },
    SafeResponseText {
        text: String,
        limit: usize,
        corrupt: bool,
    },
    Compress {
        settings: Settings,
        goals: Vec<compress::Goal>,
        reply: compress::Reply,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ValidateApiBase { settings } => {
                let trimmed = settings.api_base.trim_end_matches('/');
                Ok(match validate_api_base(trimmed) {
                    Ok(api_base) => json!({"accepted": true, "api_base": api_base}),
                    Err(_rejected) => json!({"accepted": false}),
                })
            }
            Input::QueryForTarget {
                messages,
                target_idx,
                fallback,
            } => Ok(json!({"query": query_for_target(messages, *target_idx, fallback)})),
            Input::RenderToolIntent { function_call } => {
                Ok(json!({"intent": render_tool_intent(function_call)}))
            }
            Input::FallbackQuery { messages } => {
                let (query, index) = fallback_query(messages);
                Ok(json!({"query": query, "index": index}))
            }
            Input::ReplaceTextInContent { content, new_text } => {
                let replaced = replace_text_in_content(content, new_text);
                Ok(json!({"content": serde_json::to_value(replaced).or_value_error()?}))
            }
            Input::SafeInt { value } => Ok(json!({"value": safe_int(Some(value))})),
            Input::SafeResponseText {
                text,
                limit,
                corrupt,
            } => {
                let body = (!corrupt).then_some(text.as_str());
                Ok(json!({"text": safe_response_text(body, *limit)}))
            }
            Input::Compress {
                settings,
                goals,
                reply,
            } => compress::run(settings, goals, reply),
        }
    }
}
