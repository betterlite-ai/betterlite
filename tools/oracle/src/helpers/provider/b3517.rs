use std::collections::{BTreeMap, VecDeque};
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
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::helpers::HelperError;

use transport::{block_on, recording_sandbox, request_output, requests_output, single_request};

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
    serde_json::to_value(create_body(&CreateSandbox {
        template: input.template.as_deref(),
        timeout: input.timeout,
        allow_internet_access: input.allow_internet_access,
        metadata: input.metadata.as_ref(),
        env_vars: input.env_vars.as_ref(),
        resource_limits: input.resource_limits.as_ref(),
        resource_requests: input.resource_requests.as_ref(),
        entrypoint: input.entrypoint.as_deref(),
        network_policy: input.network_policy.as_deref(),
        secure_access: input.secure_access.unwrap_or(false),
    }))
    .map_err(|error| helper_error("BodyEncoding", error))
}

fn wait_until_running_output(input: &Input) -> crate::helpers::HelperResult {
    let sandbox = recording_sandbox(input);
    let headers = headers(input)?;
    let request = WaitUntilRunning {
        sandbox_id: required_field(input.sandbox_id.as_deref(), "sandbox_id")?,
        api_base: required_field(input.api_base.as_deref(), "api_base")?,
        headers: &headers,
        ready_timeout: seconds(input.ready_timeout, "ready_timeout")?,
        poll_interval: seconds(input.poll_interval, "poll_interval")?,
    };
    block_on(sandbox.wait_until_running(&request, Some(&sandbox.http)))?;
    Ok(serde_json::json!({"requests": requests_output(&sandbox.http)?}))
}

fn wait_for_endpoint_output(input: &Input) -> crate::helpers::HelperResult {
    let ready_timeout = seconds(input.ready_timeout, "ready_timeout")?;
    let poll_interval = seconds(input.poll_interval, "poll_interval")?;
    let headers = headers(input)?;
    let target = GetEndpoint {
        sandbox_id: required_field(input.sandbox_id.as_deref(), "sandbox_id")?,
        api_base: required_field(input.api_base.as_deref(), "api_base")?,
        headers: &headers,
        use_server_proxy: input.use_server_proxy.unwrap_or(false),
    };
    let request = WaitForEndpoint {
        endpoint: target,
        ready_timeout,
        poll_interval,
    };
    let sandbox = recording_sandbox(input);
    let endpoint = block_on(sandbox.wait_for_execd_endpoint(request, Some(&sandbox.http)))?;
    Ok(serde_json::json!({
        "requests": requests_output(&sandbox.http)?,
        "endpoint": endpoint.url,
        "headers": endpoint.headers,
    }))
}

fn run_output(input: &Input) -> crate::helpers::HelperResult {
    let sandbox = recording_sandbox(input);
    let lines = block_on(sandbox.post_code(post_request(input)?, Some(&sandbox.http)))?;
    let result = parse_lines(&lines);
    let request = request_output(&single_request(&sandbox.http)?)?;
    Ok(serde_json::json!({"request": request, "result": result}))
}

fn post_request(input: &Input) -> Result<PostCode<'_>, Box<HelperError>> {
    let body = input
        .body
        .as_ref()
        .ok_or_else(|| helper_error("InvalidInput", "run needs a body"))?;
    Ok(PostCode {
        url: required_field(input.url.as_deref(), "url")?,
        headers: headers(input)?,
        body: PostCodeBody {
            code: body.code.clone(),
            context: ExecutionContext {
                language: body.context.language.clone(),
            },
        },
    })
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

fn required_field<'input>(
    value: Option<&'input str>,
    name: &str,
) -> Result<&'input str, Box<HelperError>> {
    if let Some(found) = value {
        return Ok(found);
    }
    Err(helper_error("InvalidInput", format!("{name} is required")))
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    pub(crate) template: Option<String>,
    pub(crate) timeout: Option<i64>,
    pub(crate) allow_internet_access: Option<bool>,
    pub(crate) metadata: Option<BTreeMap<String, String>>,
    pub(crate) env_vars: Option<BTreeMap<String, String>>,
    pub(crate) resource_limits: Option<BTreeMap<String, String>>,
    pub(crate) resource_requests: Option<BTreeMap<String, String>>,
    pub(crate) entrypoint: Option<Vec<String>>,
    pub(crate) network_policy: Option<Box<RawValue>>,
    pub(crate) secure_access: Option<bool>,
    pub(crate) sandbox_id: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) use_server_proxy: Option<bool>,
    pub(crate) ready_timeout: Option<f64>,
    pub(crate) poll_interval: Option<f64>,
    #[serde(default)]
    pub(crate) responses: Vec<ScriptedResponse>,
    pub(crate) response_body: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) body: Option<Body>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    CreateBody,
    WaitUntilRunning,
    WaitForExecdEndpoint,
    Run,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptedResponse {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Body {
    pub(crate) code: String,
    pub(crate) context: Context,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Context {
    pub(crate) language: String,
}

impl Input {
    pub(crate) fn scripted(&self) -> VecDeque<ScriptedResponse> {
        if self.responses.is_empty() {
            return VecDeque::from([ScriptedResponse {
                status: 200,
                body: self.response_body.clone().unwrap_or_default(),
            }]);
        }
        VecDeque::from(self.responses.clone())
    }
}

const fn default_status() -> u16 {
    200
}
