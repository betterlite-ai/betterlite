use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::sandbox::{CodeExecutionResult, ContainerHandle, HiddenParams};
use betterlite_provider::e2b::{
    CreateSandbox, DeleteSandbox, E2B_DEFAULT_DOMAIN, E2bError, E2bSandbox, RunCode, SandboxHttp,
    SandboxRef, SandboxRequest, SandboxStream,
};
use futures_util::{StreamExt as _, stream};
use secrecy::SecretString;
use serde::Serialize;

use crate::helpers::HelperError;

use input::{ContainerFields, ContainerInput, Input, Operation};

mod input;

type Environment = BTreeMap<String, String>;
type Sandbox = E2bSandbox<RecordingHttp, Environment>;

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

enum Payload {
    Create(HandleOutput),
    Run(CodeExecutionResult),
    Delete(bool),
}

struct RecordingHttp {
    status: u16,
    body: Vec<u8>,
    requests: Mutex<Vec<SandboxRequest>>,
}

#[async_trait]
impl SandboxHttp for RecordingHttp {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, E2bError> {
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
        let payload = match input.operation {
            Operation::Create => Payload::Create(create_output(&sandbox, input)?),
            Operation::Run => Payload::Run(run_output(&sandbox, input)?),
            Operation::Delete => Payload::Delete(delete_output(&sandbox, input)?),
        };
        let request = request_output(&only_request(&sandbox)?)?;
        Ok(match payload {
            Payload::Create(handle) => serde_json::json!({"request": request, "handle": handle}),
            Payload::Run(result) => serde_json::json!({"request": request, "result": result}),
            Payload::Delete(deleted) => serde_json::json!({"request": request, "deleted": deleted}),
        })
    }
}

fn create_output(sandbox: &Sandbox, input: &Input) -> Result<HandleOutput, Box<HelperError>> {
    let request = create_request(input);
    let created = block_on(sandbox.acreate_sandbox(request))?;
    Ok(HandleOutput {
        id: created.id,
        provider: created.provider,
        domain: created.domain,
    })
}

fn create_request(input: &Input) -> CreateSandbox<'_> {
    CreateSandbox {
        template: input.template.as_deref(),
        timeout: input.timeout,
        allow_internet_access: input.allow_internet_access,
        api_key: input.api_key.as_ref(),
        api_base: input.api_base.as_deref(),
    }
}

fn run_output(sandbox: &Sandbox, input: &Input) -> Result<CodeExecutionResult, Box<HelperError>> {
    let container = container_handle(input)?;
    let request = run_request(input, &container)?;
    block_on(sandbox.arun_code(request))
}

fn run_request<'request>(
    input: &'request Input,
    container: &'request ContainerHandle,
) -> Result<RunCode<'request>, Box<HelperError>> {
    let code = input
        .code
        .as_deref()
        .ok_or_else(|| helper_error("InvalidInput", "run needs a code string"))?;
    Ok(RunCode {
        container,
        code,
        env_vars: input.env_vars.as_ref(),
    })
}

fn delete_output(sandbox: &Sandbox, input: &Input) -> Result<bool, Box<HelperError>> {
    let spec = input
        .container
        .as_ref()
        .ok_or_else(|| helper_error("InvalidInput", "delete takes a container"))?;
    match spec {
        ContainerInput::Id(id) => block_on(sandbox.adelete_sandbox(DeleteSandbox {
            container: SandboxRef::Id(id.as_str()),
            api_key: input.api_key.as_ref(),
            api_base: input.api_base.as_deref(),
        })),
        ContainerInput::Handle(fields) => {
            let container = handle_from_fields(fields);
            block_on(sandbox.adelete_sandbox(DeleteSandbox {
                container: SandboxRef::Handle(&container),
                api_key: input.api_key.as_ref(),
                api_base: input.api_base.as_deref(),
            }))
        }
    }
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
            api_key: fields.api_key.clone(),
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
