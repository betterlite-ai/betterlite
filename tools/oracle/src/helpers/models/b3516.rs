use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::sandbox::{CodeExecutionResult, ContainerHandle, HiddenParams};
use betterlite_provider::e2b::{
    CreateSandbox, DeleteSandbox, E2B_DEFAULT_DOMAIN, E2bError, E2bSandbox, RunCode, SandboxHttp,
    SandboxRef, SandboxRequest, SandboxResponse, SandboxStream,
};
use futures_util::{StreamExt as _, stream};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::helpers::HelperError;

type Environment = BTreeMap<String, String>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    timeout: Option<i64>,
    #[serde(default)]
    allow_internet_access: Option<bool>,
    #[serde(default)]
    container: Option<ContainerInput>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    env_vars: Option<BTreeMap<String, String>>,
    #[serde(default = "default_response_status")]
    response_status: u16,
    #[serde(default)]
    response_body: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Create,
    Run,
    Delete,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ContainerInput {
    Id(String),
    Handle(ContainerFields),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainerFields {
    id: String,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    envd_access_token: Option<String>,
    #[serde(default)]
    traffic_access_token: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
}

const fn default_response_status() -> u16 {
    200
}

#[derive(Serialize)]
struct RequestOutput {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    json: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct HandleOutput {
    id: String,
    provider: String,
    domain: Option<String>,
}

#[derive(Serialize)]
struct CreateOutput {
    request: RequestOutput,
    handle: HandleOutput,
}

#[derive(Serialize)]
struct RunOutput {
    request: RequestOutput,
    result: CodeExecutionResult,
}

#[derive(Serialize)]
struct DeleteOutput {
    request: RequestOutput,
    deleted: bool,
}

struct RecordingHttp {
    status: u16,
    body: Vec<u8>,
    requests: Mutex<Vec<SandboxRequest>>,
}

#[async_trait]
impl SandboxHttp for RecordingHttp {
    async fn send_buffered(&self, request: SandboxRequest) -> Result<SandboxResponse, E2bError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(SandboxResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }

    async fn send_streaming(&self, request: SandboxRequest) -> Result<SandboxStream, E2bError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(SandboxStream {
            status: self.status,
            body: stream::iter(vec![Ok::<Vec<u8>, E2bError>(self.body.clone())]).boxed(),
        })
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let sandbox = E2bSandbox {
            http: RecordingHttp {
                status: input.response_status,
                body: input.response_body.as_bytes().to_vec(),
                requests: Mutex::new(Vec::new()),
            },
            environment: Environment::new(),
        };
        match input.operation {
            Operation::Create => create_output(&sandbox, input),
            Operation::Run => run_output(&sandbox, input),
            Operation::Delete => delete_output(&sandbox, input),
        }
    }
}

type Sandbox = E2bSandbox<RecordingHttp, Environment>;

fn create_output(sandbox: &Sandbox, input: &Input) -> crate::helpers::HelperResult {
    let created = block_on(sandbox.acreate_sandbox(CreateSandbox {
        template: input.template.as_deref(),
        timeout: input.timeout,
        allow_internet_access: input.allow_internet_access,
        api_key: input.api_key.as_deref(),
        api_base: input.api_base.as_deref(),
    }))?;
    let output = CreateOutput {
        request: request_output(&only_request(sandbox)?)?,
        handle: HandleOutput {
            id: created.id,
            provider: created.provider,
            domain: created.domain,
        },
    };
    serde_json::to_value(output).map_err(|error| helper_error("SerializationError", error))
}

fn run_output(sandbox: &Sandbox, input: &Input) -> crate::helpers::HelperResult {
    let code = input
        .code
        .as_deref()
        .ok_or_else(|| helper_error("InvalidInput", "run takes a code string"))?;
    let container = container_handle(input)?;
    let result = block_on(sandbox.arun_code(RunCode {
        container: &container,
        code,
        env_vars: input.env_vars.as_ref(),
    }))?;
    let output = RunOutput {
        request: request_output(&only_request(sandbox)?)?,
        result,
    };
    serde_json::to_value(output).map_err(|error| helper_error("SerializationError", error))
}

fn delete_output(sandbox: &Sandbox, input: &Input) -> crate::helpers::HelperResult {
    let spec = input
        .container
        .as_ref()
        .ok_or_else(|| helper_error("InvalidInput", "delete takes a container"))?;
    let deleted = match spec {
        ContainerInput::Id(id) => block_on(sandbox.adelete_sandbox(DeleteSandbox {
            container: SandboxRef::Id(id.as_str()),
            api_key: input.api_key.as_deref(),
            api_base: input.api_base.as_deref(),
        }))?,
        ContainerInput::Handle(fields) => {
            let container = handle_from_fields(fields);
            block_on(sandbox.adelete_sandbox(DeleteSandbox {
                container: SandboxRef::Handle(&container),
                api_key: input.api_key.as_deref(),
                api_base: input.api_base.as_deref(),
            }))?
        }
    };
    let output = DeleteOutput {
        request: request_output(&only_request(sandbox)?)?,
        deleted,
    };
    serde_json::to_value(output).map_err(|error| helper_error("SerializationError", error))
}

fn container_handle(input: &Input) -> Result<ContainerHandle, Box<HelperError>> {
    match input
        .container
        .as_ref()
        .ok_or_else(|| helper_error("InvalidInput", "the operation needs a container"))?
    {
        ContainerInput::Id(id) => Ok(ContainerHandle {
            id: id.clone(),
            provider: "e2b".to_owned(),
            domain: Some(E2B_DEFAULT_DOMAIN.to_owned()),
            hidden_params: HiddenParams::default(),
        }),
        ContainerInput::Handle(fields) => Ok(handle_from_fields(fields)),
    }
}

fn handle_from_fields(fields: &ContainerFields) -> ContainerHandle {
    ContainerHandle {
        id: fields.id.clone(),
        provider: "e2b".to_owned(),
        domain: fields.domain.clone(),
        hidden_params: HiddenParams {
            envd_access_token: fields.envd_access_token.as_deref().map(SecretString::from),
            traffic_access_token: fields
                .traffic_access_token
                .as_deref()
                .map(SecretString::from),
            api_key: fields.api_key.as_deref().map(SecretString::from),
            api_base: fields.api_base.clone(),
        },
    }
}

fn only_request(sandbox: &Sandbox) -> Result<SandboxRequest, Box<HelperError>> {
    let requests = sandbox
        .http
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match (requests.first(), requests.len()) {
        (Some(request), 1) => Ok(request.clone()),
        _ => Err(helper_error("UnexpectedRequests", requests.len())),
    }
}

fn request_output(request: &SandboxRequest) -> Result<RequestOutput, Box<HelperError>> {
    let mut headers = BTreeMap::new();
    for (name, value) in &request.headers {
        let text = value
            .to_str()
            .map_err(|error| helper_error("HeaderEncoding", error))?;
        headers.insert(name.as_str().to_owned(), text.to_owned());
    }
    let json = match request.body.as_slice() {
        [] => None,
        body => Some(
            serde_json::from_slice(body)
                .map_err(|error| helper_error("RequestBodyEncoding", error))?,
        ),
    };
    Ok(RequestOutput {
        method: request.method.as_str().to_owned(),
        url: request.url.to_string(),
        headers,
        json,
    })
}

fn block_on<Output, Future>(future: Future) -> Result<Output, Box<HelperError>>
where
    Future: std::future::Future<Output = Result<Output, E2bError>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error))?;
    match runtime.block_on(future) {
        Ok(output) => Ok(output),
        Err(error) => Err(helper_error("E2bError", error)),
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
