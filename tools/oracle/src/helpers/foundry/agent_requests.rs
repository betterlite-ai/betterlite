use betterlite_foundry::agent_requests::endpoint::endpoint_url;
use betterlite_foundry::agent_requests::error::{AgentRequestError, check_response};
use betterlite_foundry::agent_requests::params::SUPPORTED_OPENAI_PARAMS;
use betterlite_foundry::agent_requests::prepare::validate_environment;
use betterlite_foundry::agent_requests::request::{agent_id, complete_url};
use betterlite_foundry::agent_requests::stream::{
    HAS_CUSTOM_STREAM_WRAPPER, SHOULD_FAKE_STREAM, SUPPORTS_STREAM_PARAM_IN_BODY,
};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde_json::{Value, json};

mod answers;
mod input;
mod secrets;
mod tests;

use answers::{
    answer, outcome, prepared_call, resolved_provider, secret_headers, segment, shown_headers,
    transformed_request,
};
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::AgentIds(calls) => answer(calls, |call| {
                outcome(agent_id(&call.model, &call.optional_params))
            }),
            // the agent route copies no non-default parameter, so the mapped body is optional_params as given
            Input::Params(calls) => answer(calls, |call| {
                Ok(json!({
                    "supported": SUPPORTED_OPENAI_PARAMS,
                    "mapped": call.optional_params,
                }))
            }),
            Input::ProviderInfos(calls) => answer(calls, resolved_provider),
            Input::CompleteUrls(calls) => answer(calls, |call| {
                outcome(complete_url(call.api_base.as_deref()))
            }),
            Input::Requests(calls) => answer(calls, transformed_request),
            Input::Preparations(calls) => answer(calls, prepared_call),
            Input::Validations(calls) => answer(calls, |call| {
                let key = call.api_key.as_deref().map(SecretString::from);
                let headers = validate_environment(&secret_headers(&call.headers), key.as_ref());
                Ok(json!({"ok": true, "value": {"headers": shown_headers(&headers)}}))
            }),
            Input::Checks(calls) => answer(calls, |call| {
                match check_response(call.status, &call.body, &call.expected, &call.error_msg) {
                    Ok(()) => Ok(json!({"ok": true})),
                    Err(error) => outcome::<Value>(Err(error)),
                }
            }),
            // get_error_class ignores the headers it is handed, so the error keeps only the status and the message
            Input::Errors(calls) => answer(calls, |call| {
                let error = AgentRequestError::Upstream {
                    status: call.status,
                    message: call.message.clone(),
                };
                Ok(json!({"status": error.status(), "headers_dropped": true}))
            }),
            Input::Paths(calls) => answer(calls, |call| {
                outcome(endpoint_url(
                    segment(call),
                    &call.api_base,
                    &call.api_version,
                ))
            }),
            Input::Flags(calls) => answer(calls, |call| {
                Ok(json!({
                    "model": call.model,
                    "stream": call.stream,
                    "should_fake_stream": SHOULD_FAKE_STREAM,
                    "has_custom_stream_wrapper": HAS_CUSTOM_STREAM_WRAPPER,
                    "supports_stream_param_in_request_body": SUPPORTS_STREAM_PARAM_IN_BODY,
                    "transform_response_is_noop": true,
                }))
            }),
        }
    }
}
