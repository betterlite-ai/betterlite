use std::sync::PoisonError;

use betterlite_anthropic::anthropic_http_stream::call::{OpenedStream, StreamBuild, StreamClients};
use betterlite_anthropic::anthropic_http_stream::completion::{
    CompletionStream, acompletion_stream_function,
};
use betterlite_anthropic::anthropic_http_stream::error::StreamOpenError;
use betterlite_anthropic::headers::response::process_anthropic_headers;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::calls::{Outcome, TIMEOUT, attempt, transport};
use super::fixture::{FixedCounter, FixedIds, HelperFailure, SilentLogger};
use super::support::{HelperValue, header_map, options_value, post_value, selected};
use super::{Input, MAX_EVENT_BYTES, value_error};

struct Replied {
    post: Value,
    polled: bool,
    opened: OpenedStream<HelperFailure, FixedCounter>,
}

fn replied(input: &Input, synchronous: bool) -> Result<Replied, Box<HelperError>> {
    let attempt = attempt(input, synchronous, Outcome::Reply)?;
    let post = attempt.posts.first().map_or(Value::Null, |post| {
        post_value(post, &input.request_header_names)
    });
    let polled = attempt.polled;
    let opened = attempt
        .result
        .map_err(|error| value_error(error.to_string()))?;
    Ok(Replied {
        post,
        polled,
        opened,
    })
}

pub(super) fn stream_post(input: &Input) -> HelperResult {
    Ok(json!({
        "async": opened_value(input, false)?,
        "sync": opened_value(input, true)?,
    }))
}

fn opened_value(input: &Input, synchronous: bool) -> HelperValue {
    let reply = replied(input, synchronous)?;
    Ok(json!({
        "post": reply.post,
        "stream_options": options_value(&reply.opened.completion_stream.converter.state),
        "response_headers": selected(&reply.opened.headers, &input.response_header_names),
    }))
}

pub(super) fn handshake_error(input: &Input) -> HelperResult {
    Ok(json!({
        "async": error_value(input, false, Outcome::Status)?,
        "sync": error_value(input, true, Outcome::Status)?,
    }))
}

pub(super) fn sync_status_guard(input: &Input) -> HelperResult {
    let reply = replied(input, false)?;
    let options = options_value(&reply.opened.completion_stream.converter.state);
    let rejected = error_value(input, true, Outcome::Reply)?;
    let opened = json!({"opened": true, "response_read": reply.polled, "stream_options": options});
    Ok(json!({"async": opened, "sync": rejected}))
}

pub(super) fn non_http_error(input: &Input) -> HelperResult {
    Ok(json!({
        "async_unknown": error_value(input, false, Outcome::Unknown)?,
        "async_known": error_value(input, false, Outcome::Timeout)?,
        "sync_unknown": error_value(input, true, Outcome::Unknown)?,
        "sync_known": error_value(input, true, Outcome::Timeout)?,
    }))
}

// the answer for a call that raised: LiteLLM's AnthropicError fields, or the Timeout it re-raises
fn error_value(input: &Input, synchronous: bool, outcome: Outcome) -> HelperValue {
    let Err(error) = attempt(input, synchronous, outcome)?.result else {
        return Err(value_error("the call opened a stream".to_owned()));
    };
    match error {
        StreamOpenError::Anthropic {
            status_code,
            message,
            headers,
        } => Ok(json!({
            "status": status_code,
            "message": message,
            "headers": selected(&headers.unwrap_or_default(), &input.response_header_names),
        })),
        StreamOpenError::Unknown { source } => Ok(json!({
            "class": "AnthropicError",
            "status": 500,
            "message": source.to_string(),
        })),
        StreamOpenError::Reraised {
            source: HelperFailure::Timeout,
        } => Ok(json!({ "class": "Timeout", "status": 408 })),
        // any other raised error carries the fixture's own diagnostic as its display text
        other => Err(value_error(other.to_string())),
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
    let reply = replied(input, false)?;
    Ok(json!({
        "post": reply.post,
        "raw": selected(&reply.opened.headers, &input.response_header_names),
        "projected": process_anthropic_headers(&reply.opened.headers),
    }))
}
