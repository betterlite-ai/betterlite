use std::sync::PoisonError;

use betterlite_anthropic::anthropic_http_stream::call::{StreamBuild, StreamClients};
use betterlite_anthropic::anthropic_http_stream::completion::{
    CompletionStream, acompletion_stream_function,
};
use betterlite_anthropic::anthropic_http_stream::error::StreamOpenError;
use betterlite_anthropic::headers::response::process_anthropic_headers;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::calls::{Outcome, TIMEOUT, attempt, transport};
use super::fixture::{FixedCounter, FixedIds, HelperFailure, SilentLogger};
use super::support::{HelperValue, header_map, options_value, post_value, selected};
use super::{Input, MAX_EVENT_BYTES, value_error};

pub(super) fn stream_post(input: &Input) -> HelperResult {
    Ok(json!({
        "async": opened_value(input, false)?,
        "sync": opened_value(input, true)?,
    }))
}

fn opened_value(input: &Input, synchronous: bool) -> HelperValue {
    let attempt = attempt(input, synchronous, Outcome::Reply)?;
    let post = attempt.posts.first().map_or(Value::Null, |post| {
        post_value(post, &input.request_header_names)
    });
    let opened = attempt
        .result
        .map_err(|error| value_error(error.to_string()))?;
    Ok(json!({
        "post": post,
        "stream_options": options_value(&opened.completion_stream.converter.state),
        "response_headers": selected(&opened.headers, &input.response_header_names),
    }))
}

pub(super) fn handshake_error(input: &Input) -> HelperResult {
    Ok(json!({
        "async": status_error_value(input, false, Outcome::Status)?,
        "sync": status_error_value(input, true, Outcome::Status)?,
    }))
}

pub(super) fn sync_status_guard(input: &Input) -> HelperResult {
    let attempt = attempt(input, false, Outcome::Reply)?;
    let opened = attempt
        .result
        .map_err(|error| value_error(error.to_string()))?;
    Ok(json!({
        "async": {
            "opened": true,
            "response_read": attempt.polled,
            "stream_options": options_value(&opened.completion_stream.converter.state),
        },
        "sync": status_error_value(input, true, Outcome::Reply)?,
    }))
}

fn status_error_value(input: &Input, synchronous: bool, outcome: Outcome) -> HelperValue {
    match attempt(input, synchronous, outcome)?.result {
        Err(StreamOpenError::Anthropic {
            status_code,
            message,
            headers,
        }) => Ok(json!({
            "status": status_code,
            "message": message,
            "headers": selected(&headers.unwrap_or_default(), &input.response_header_names),
        })),
        Ok(_) => Err(value_error("the call opened a stream".to_owned())),
        Err(error) => Err(value_error(error.to_string())),
    }
}

pub(super) fn non_http_error(input: &Input) -> HelperResult {
    Ok(json!({
        "async_unknown": failure_value(input, false, Outcome::Unknown)?,
        "async_known": failure_value(input, false, Outcome::Timeout)?,
        "sync_unknown": failure_value(input, true, Outcome::Unknown)?,
        "sync_known": failure_value(input, true, Outcome::Timeout)?,
    }))
}

fn failure_value(input: &Input, synchronous: bool, outcome: Outcome) -> HelperValue {
    match attempt(input, synchronous, outcome)?.result {
        Err(StreamOpenError::Unknown { source }) => Ok(json!({
            "class": "AnthropicError",
            "status": 500,
            "message": source.to_string(),
        })),
        Err(StreamOpenError::Reraised { source }) => match &source {
            HelperFailure::Timeout => Ok(json!({ "class": "Timeout", "status": 408 })),
            HelperFailure::Unknown { message } => Err(value_error(message.clone())),
            HelperFailure::Exhausted => {
                Err(value_error("the fixture has no reply left".to_owned()))
            }
        },
        Ok(_) => Err(value_error("the call opened a stream".to_owned())),
        Err(error) => Err(value_error(error.to_string())),
    }
}

pub(super) fn stream_flag_and_context(input: &Input) -> HelperResult {
    let client = transport(input, Outcome::Reply)?;
    let logger = SilentLogger;
    let clients = StreamClients {
        supplied: Some(&client),
        module_level_aclient: &client,
        module_level_client: &client,
        logger: &logger,
    };
    let build = StreamBuild {
        ids: &FixedIds,
        counter: FixedCounter,
        max_event_bytes: MAX_EVENT_BYTES,
        max_error_body_bytes: MAX_EVENT_BYTES,
    };
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let wrapper = runtime
        .block_on(acompletion_stream_function(
            clients,
            CompletionStream {
                model: &input.model,
                api_base: &input.api_base,
                headers: header_map(&input.headers)?,
                data: input.request_data.clone(),
                messages: &input.messages,
                timeout: TIMEOUT,
                json_mode: input.json_mode,
                optional_params: &input.optional_params,
                litellm_params: &input.litellm_params,
            },
            build,
        ))
        .map_err(|error| value_error(error.to_string()))?;
    let posts = std::mem::take(&mut *client.posts.lock().unwrap_or_else(PoisonError::into_inner));
    let post = posts.first().map_or(Value::Null, |post| {
        post_value(post, &input.request_header_names)
    });
    Ok(json!({
        "wrapper": {
            "model": &wrapper.model,
            "custom_llm_provider": wrapper.custom_llm_provider,
        },
        "request": post,
        "stream_options": options_value(&wrapper.completion_stream.converter.state),
    }))
}

pub(super) fn response_headers(input: &Input) -> HelperResult {
    let attempt = attempt(input, false, Outcome::Reply)?;
    let post = attempt.posts.first().map_or(Value::Null, |post| {
        post_value(post, &input.request_header_names)
    });
    let opened = attempt
        .result
        .map_err(|error| value_error(error.to_string()))?;
    Ok(json!({
        "post": post,
        "raw": selected(&opened.headers, &input.response_header_names),
        "projected": process_anthropic_headers(&opened.headers),
    }))
}
