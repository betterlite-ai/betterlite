use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::usage::AnthropicUsage;
use betterlite_bedrock::claude_invoke::request::ClaudeInvokeRequest;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};

use super::fixture::{Fixture, Session};
use super::input::{Input, MapCall, ModelCost, ResponseCall, TransformCall};
use super::mock::MapFetcher;
use super::response::response;

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockClaudeInvoke".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn headers(entries: &BTreeMap<String, String>) -> AnthropicHeaders {
    let mut headers = AnthropicHeaders::default();
    for (name, value) in entries {
        headers.values.insert(name.clone(), value.clone());
    }
    headers
}

fn transforms(model_cost: &ModelCost, calls: &[TransformCall]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let mut session = Session::new(&fixture).map_err(failure)?;
    let defaults = OptionDefaults::default();
    let catalog = ModelCatalog::parse(b"{}", UnknownFields::Warn).map_err(failure)?;
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let fetcher = MapFetcher::new(&call.fetch);
        let request = ClaudeInvokeRequest {
            model: &call.model,
            messages: call.messages.clone(),
            optional_params: call.optional_params.clone(),
            litellm_params: call.litellm_params.clone(),
            headers: headers(&call.headers),
            defaults: &defaults,
            catalog: &catalog,
            default_chat_max_tokens: 4096,
            modify_params: false,
            global_drop_params: false,
        };
        let prepared = session
            .transform(&request, &fetcher, call.inline)
            .map_err(failure)?;
        let body = serde_json::to_value(&prepared.body).map_err(failure)?;
        answers.push(json!({
            "body": body,
            "fetches": fetcher.calls(),
        }));
    }
    Ok(json!({ "calls": answers }))
}

fn maps(model_cost: &ModelCost, calls: &[MapCall]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let mut session = Session::new(&fixture).map_err(failure)?;
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let mut optional_params = call.optional_params.clone();
        session
            .map_params(
                &call.model,
                call.drop_params,
                &call.non_default_params,
                &mut optional_params,
            )
            .map_err(failure)?;
        answers.push(json!({ "optional_params": optional_params }));
    }
    Ok(json!({ "calls": answers }))
}

fn usage_view(usage: &AnthropicUsage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
        "cache_read_input_tokens": usage.cache_read_input_tokens,
        "reasoning_tokens": usage.completion_tokens_details.reasoning,
        "text_tokens": usage.completion_tokens_details.text,
    })
}

fn responses(calls: &[ResponseCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let envelope = response(
            &call.raw_text,
            &call.messages,
            &call.optional_params,
            &call.litellm_params,
            call.now,
        )?;
        let choice = envelope.choices.first();
        let usage = envelope.usage.as_ref().map(usage_view);
        answers.push(json!({
            "id": envelope.id,
            "model": envelope.model,
            "finish_reason": choice.and_then(|chosen| chosen.finish_reason.clone()),
            "content": choice.and_then(|chosen| chosen.message.content.clone()),
            "usage": usage,
        }));
    }
    Ok(json!({ "calls": answers }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Transform { model_cost, calls } => transforms(model_cost, calls),
        Input::MapParams { model_cost, calls } => maps(model_cost, calls),
        Input::Response { calls } => responses(calls),
    }
}
