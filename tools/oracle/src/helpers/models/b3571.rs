use betterlite_models::passthrough_policies::generic_timeout::resolve_passthrough_timeout;
use betterlite_models::passthrough_policies::header_forwarding::forward_headers_from_request;
use betterlite_models::passthrough_policies::query_merge::{
    QueryParameters, merge_query_parameters,
};
use betterlite_models::passthrough_policies::sdk_timeout::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeouts, TimeoutValue, resolve_sdk_timeout,
};
use serde_json::json;
use url::Url;

use crate::helpers::HelperError;

use input::{DeploymentFields, Input, Operation, RequestFields, RouterFields, ValueInput};

mod input;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.operation {
            Operation::QueryMerge => query_output(input),
            Operation::ForwardHeaders => header_output(input),
            Operation::ProxyTimeout => Ok(json!(resolve_passthrough_timeout(
                input.endpoint_timeout,
                input.settings_timeout,
            ))),
            Operation::SdkTimeout => sdk_output(input),
        }
    }
}

fn query_output(input: &Input) -> crate::helpers::HelperResult {
    let existing_url = input
        .existing_url
        .as_deref()
        .ok_or_else(|| helper_error("InvalidInput", "query_merge needs existing_url"))?;
    let url = Url::parse(existing_url).map_err(|error| helper_error("InvalidUrl", error))?;
    let empty = QueryParameters::new();
    let request = input.request_query_params.as_ref().unwrap_or(&empty);
    let merged = merge_query_parameters(&url, request, input.default_query_params.as_ref());
    serde_json::to_value(merged).map_err(|error| helper_error("Serialize", error))
}

fn header_output(input: &Input) -> crate::helpers::HelperResult {
    let request = input.request_headers.clone().unwrap_or_default();
    let custom = input.custom_headers.clone().unwrap_or_default();
    let merged =
        forward_headers_from_request(&request, custom, input.forward_headers.unwrap_or(false));
    serde_json::to_value(merged).map_err(|error| helper_error("Serialize", error))
}

fn sdk_output(input: &Input) -> crate::helpers::HelperResult {
    let request = request_timeouts(input.request.as_ref());
    let deployment = deployment_timeouts(input.deployment.as_ref());
    let router = router_timeouts(input.router.as_ref());
    let seconds = resolve_sdk_timeout(
        &request,
        &deployment,
        &router,
        input.configured_timeout,
        input.settings_timeout,
    )
    .map_err(|error| helper_error("InvalidSeconds", error))?;
    Ok(json!(seconds))
}

fn request_timeouts(fields: Option<&RequestFields>) -> RequestTimeouts {
    fields.map_or_else(RequestTimeouts::default, |source| RequestTimeouts {
        stream: source.stream,
        stream_timeout: timeout_value(source.stream_timeout.as_ref()),
        timeout: timeout_value(source.timeout.as_ref()),
        request_timeout: timeout_value(source.request_timeout.as_ref()),
    })
}

fn deployment_timeouts(fields: Option<&DeploymentFields>) -> DeploymentTimeouts {
    fields.map_or_else(DeploymentTimeouts::default, |source| DeploymentTimeouts {
        stream_timeout: timeout_value(source.stream_timeout.as_ref()),
        timeout: timeout_value(source.timeout.as_ref()),
        request_timeout: timeout_value(source.request_timeout.as_ref()),
    })
}

fn router_timeouts(fields: Option<&RouterFields>) -> RouterTimeouts {
    fields.map_or_else(RouterTimeouts::default, |source| RouterTimeouts {
        stream_timeout: timeout_value(source.stream_timeout.as_ref()),
        timeout: timeout_value(source.timeout.as_ref()),
    })
}

fn timeout_value(input: Option<&ValueInput>) -> Option<TimeoutValue> {
    match input {
        Some(ValueInput::Number(value)) => Some(TimeoutValue::Number(*value)),
        Some(ValueInput::Text(text)) => Some(TimeoutValue::Text(text.clone())),
        None => None,
    }
}

fn helper_error<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
