use std::time::Duration;

use betterlite_provider::opensandbox::helpers::create::{CreateSandbox, create_body};
use betterlite_provider::opensandbox::helpers::poll::{
    GetEndpoint, WaitForEndpoint, WaitUntilRunning,
};
use betterlite_provider::opensandbox::helpers::request::{
    ExecutionContext, PostCode, PostCodeBody,
};
use betterlite_provider::opensandbox::helpers::sse::parse_lines;
use http::{HeaderMap, HeaderName, HeaderValue};

use crate::helpers::HelperError;

use input::{Body, Input, Operation};
use output::{request_output, requests_output, single_request};
use transport::{block_on, recording_sandbox};

mod input;
mod output;
mod transport;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.operation {
            Operation::CreateBody => create_output(input),
            Operation::WaitUntilRunning => wait_until_running_output(input),
            Operation::WaitForExecdEndpoint => wait_for_endpoint_output(input),
            Operation::Run => run_output(input),
        }
    }
}

fn create_output(input: &Input) -> crate::helpers::HelperResult {
    let entrypoint = input.entrypoint.as_deref();
    let body = create_body(&CreateSandbox {
        template: input.template.as_deref(),
        timeout: input.timeout,
        allow_internet_access: input.allow_internet_access,
        metadata: input.metadata.as_ref(),
        env_vars: input.env_vars.as_ref(),
        resource_limits: input.resource_limits.as_ref(),
        resource_requests: input.resource_requests.as_ref(),
        entrypoint,
        network_policy: input.network_policy.as_deref(),
        secure_access: input.secure_access.unwrap_or(false),
    });
    serde_json::to_value(body).map_err(|error| helper_error("BodyEncoding", error))
}

fn wait_until_running_output(input: &Input) -> crate::helpers::HelperResult {
    let sandbox = recording_sandbox(input);
    let headers = headers(input)?;
    let request = WaitUntilRunning {
        sandbox_id: input
            .sandbox_id
            .as_deref()
            .ok_or_else(|| helper_error("InvalidInput", "sandbox_id is required"))?,
        api_base: input
            .api_base
            .as_deref()
            .ok_or_else(|| helper_error("InvalidInput", "api_base is required"))?,
        headers: &headers,
        ready_timeout: seconds(input.ready_timeout, "ready_timeout")?,
        poll_interval: seconds(input.poll_interval, "poll_interval")?,
    };
    block_on(sandbox.wait_until_running(&request, Some(&sandbox.http)))?;
    Ok(serde_json::json!({"requests": requests_output(&sandbox.http)?}))
}

fn wait_for_endpoint_output(input: &Input) -> crate::helpers::HelperResult {
    let sandbox = recording_sandbox(input);
    let headers = headers(input)?;
    let request = WaitForEndpoint {
        endpoint: GetEndpoint {
            sandbox_id: input
                .sandbox_id
                .as_deref()
                .ok_or_else(|| helper_error("InvalidInput", "sandbox_id is required"))?,
            api_base: input
                .api_base
                .as_deref()
                .ok_or_else(|| helper_error("InvalidInput", "api_base is required"))?,
            headers: &headers,
            use_server_proxy: input.use_server_proxy.unwrap_or(false),
        },
        ready_timeout: seconds(input.ready_timeout, "ready_timeout")?,
        poll_interval: seconds(input.poll_interval, "poll_interval")?,
    };
    let endpoint = block_on(sandbox.wait_for_execd_endpoint(&request, Some(&sandbox.http)))?;
    Ok(serde_json::json!({
        "requests": requests_output(&sandbox.http)?,
        "endpoint": endpoint.url,
        "headers": endpoint.headers,
    }))
}

fn run_output(input: &Input) -> crate::helpers::HelperResult {
    let sandbox = recording_sandbox(input);
    let body = input
        .body
        .as_ref()
        .ok_or_else(|| helper_error("InvalidInput", "run needs a body"))?;
    let post_body = post_body(body);
    let post = PostCode {
        url: input
            .url
            .as_deref()
            .ok_or_else(|| helper_error("InvalidInput", "url is required"))?,
        headers: headers(input)?,
        body: &post_body,
    };
    let lines = block_on(sandbox.post_code(post, Some(&sandbox.http)))?;
    let result = parse_lines(&lines);
    let request = request_output(&single_request(&sandbox.http)?)?;
    Ok(serde_json::json!({"request": request, "result": result}))
}

fn post_body(body: &Body) -> PostCodeBody {
    PostCodeBody {
        code: body.code.clone(),
        context: ExecutionContext {
            language: body.context.language.clone(),
        },
    }
}

fn headers(input: &Input) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    if let Some(entries) = input.headers.as_ref() {
        for (name, value) in entries {
            let header_name = HeaderName::try_from(name.as_str())
                .map_err(|error| helper_error("InvalidHeaderName", error))?;
            let header_value = HeaderValue::try_from(value.as_str())
                .map_err(|error| helper_error("InvalidHeaderValue", error))?;
            map.insert(header_name, header_value);
        }
    }
    Ok(map)
}

fn seconds(value: Option<f64>, name: &str) -> Result<Duration, Box<HelperError>> {
    value.map_or_else(
        || Err(helper_error("InvalidInput", format!("{name} is required"))),
        |given| {
            Duration::try_from_secs_f64(given)
                .map_err(|error| helper_error("InvalidDuration", error))
        },
    )
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
