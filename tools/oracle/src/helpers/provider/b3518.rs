use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::sandbox::{ContainerHandle, HiddenParams};
use betterlite_provider::opensandbox::helpers::poll::Endpoint;
use betterlite_provider::opensandbox::lifecycle::{
    ContainerRef, CreateSandboxRequest, DeleteSandbox, RunCode, SandboxHandle, SandboxOptions,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::helpers::HelperError;

use transport::{block_on, handle_output, recording_sandbox, requests_output};

mod transport;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let sandbox = recording_sandbox(input);
        let api_key = input.api_key.as_deref().map(SecretString::from);
        let options = sandbox_options(input, api_key.as_ref())?;
        match input.operation {
            Operation::Create => {
                let handle =
                    block_on(sandbox.acreate_sandbox(prepared_create(input, options), None))?;
                Ok(serde_json::json!({
                    "handle": handle_output(&handle),
                    "requests": requests_output(&sandbox.http)?,
                }))
            }
            Operation::Run => {
                let container = container_input(input)?;
                let result = block_on(
                    sandbox.arun_code(prepared_run(input, container.reference(), options)?, None),
                )?;
                Ok(serde_json::json!({
                    "requests": requests_output(&sandbox.http)?,
                    "result": result,
                }))
            }
            Operation::Delete => {
                let container = container_input(input)?;
                let deleted = block_on(sandbox.adelete_sandbox(
                    DeleteSandbox {
                        container: container.reference(),
                        options,
                    },
                    None,
                ))?;
                Ok(serde_json::json!({
                    "deleted": deleted,
                    "requests": requests_output(&sandbox.http)?,
                }))
            }
        }
    }
}

fn sandbox_options<'request>(
    input: &'request Input,
    api_key: Option<&'request SecretString>,
) -> Result<SandboxOptions<'request>, Box<HelperError>> {
    Ok(SandboxOptions {
        api_key,
        api_base: input.api_base.as_deref(),
        use_server_proxy: input.use_server_proxy.unwrap_or(false),
        ready_timeout: seconds(input.ready_timeout)?,
        poll_interval: seconds(input.poll_interval)?,
    })
}

fn prepared_create<'request>(
    input: &'request Input,
    options: SandboxOptions<'request>,
) -> CreateSandboxRequest<'request> {
    CreateSandboxRequest {
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
        options,
    }
}

fn prepared_run<'request>(
    input: &'request Input,
    container: ContainerRef<'request>,
    options: SandboxOptions<'request>,
) -> Result<RunCode<'request>, Box<HelperError>> {
    Ok(RunCode {
        container,
        code: required(input.code.as_deref(), "code")?,
        language: input.language.as_deref(),
        options,
    })
}

enum Container {
    Cached(SandboxHandle),
    Bare(ContainerHandle),
    Id(String),
}

impl Container {
    fn reference(&self) -> ContainerRef<'_> {
        match self {
            Self::Cached(handle) => ContainerRef::Cached(handle),
            Self::Bare(handle) => ContainerRef::Handle(handle),
            Self::Id(id) => ContainerRef::Id(id),
        }
    }
}

fn container_input(input: &Input) -> Result<Container, Box<HelperError>> {
    let Some(chosen) = input.container.as_ref() else {
        return Err(helper_error("InvalidInput", "the call needs a container"));
    };
    let handle = match chosen {
        ContainerInput::Id(id) => return Ok(Container::Id(id.clone())),
        ContainerInput::Handle(handle) => handle,
    };
    let hidden = handle.hidden_params.as_ref();
    let container = ContainerHandle {
        id: handle.id.clone(),
        provider: handle
            .provider
            .clone()
            .unwrap_or_else(|| "opensandbox".to_owned()),
        domain: handle.domain.clone(),
        hidden_params: HiddenParams {
            envd_access_token: None,
            traffic_access_token: None,
            api_key: hidden
                .and_then(|fields| fields.api_key.clone())
                .map(SecretString::from),
            api_base: hidden.and_then(|fields| fields.api_base.clone()),
        },
    };
    let Some(url) = hidden.and_then(|fields| fields.execd_endpoint.clone()) else {
        return Ok(Container::Bare(container));
    };
    Ok(Container::Cached(SandboxHandle {
        container,
        execd_endpoint: Endpoint {
            url,
            headers: hidden
                .and_then(|fields| fields.execd_headers.clone())
                .unwrap_or_default(),
        },
        use_server_proxy: hidden
            .and_then(|fields| fields.use_server_proxy)
            .unwrap_or(false),
    }))
}

fn required<'input>(
    value: Option<&'input str>,
    name: &str,
) -> Result<&'input str, Box<HelperError>> {
    let Some(found) = value else {
        return Err(helper_error("InvalidInput", format!("{name} is required")));
    };
    Ok(found)
}

fn seconds(value: Option<f64>) -> Result<Option<Duration>, Box<HelperError>> {
    value
        .map(|given| {
            Duration::try_from_secs_f64(given)
                .map_err(|error| helper_error("InvalidDuration", error))
        })
        .transpose()
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
    pub(crate) use_server_proxy: Option<bool>,
    pub(crate) ready_timeout: Option<f64>,
    pub(crate) poll_interval: Option<f64>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) container: Option<ContainerInput>,
    pub(crate) code: Option<String>,
    pub(crate) language: Option<String>,
    #[serde(default)]
    pub(crate) responses: Vec<ScriptedResponse>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Create,
    Run,
    Delete,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum ContainerInput {
    Handle(HandleInput),
    Id(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandleInput {
    pub(crate) id: String,
    pub(crate) provider: Option<String>,
    pub(crate) domain: Option<String>,
    pub(crate) hidden_params: Option<HiddenInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HiddenInput {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) execd_endpoint: Option<String>,
    pub(crate) execd_headers: Option<BTreeMap<String, String>>,
    pub(crate) use_server_proxy: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptedResponse {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
}

const fn default_status() -> u16 {
    200
}
