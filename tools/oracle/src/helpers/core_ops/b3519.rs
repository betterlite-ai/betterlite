mod input;
mod support;

use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::sandbox_dispatch::{SandboxToolConfig, SandboxToolEntry};
use betterlite_provider::e2b::E2bSandbox;
use betterlite_provider::opensandbox::helpers::OpenSandbox;
use betterlite_provider::sandbox_dispatch::lifecycle::SandboxDispatch;
use betterlite_provider::sandbox_dispatch::registry::SandboxToolRegistry;
use betterlite_provider::sandbox_dispatch::request::{
    CreateSandbox, DeleteSandbox, RunCode, SandboxCall, SandboxContainerRef,
};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

use input::{CreateInput, DeleteInput, Input, RegistryInput, RunInput};
use support::{Environment, FakeHttp, StepClock, container_value, secrets};

type Dispatch = SandboxDispatch<
    E2bSandbox<FakeHttp, Environment>,
    OpenSandbox<FakeHttp, Environment, StepClock>,
>;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(command: &Self::Input) -> HelperResult {
        match command {
            Input::Registry(registry) => registry_answer(registry),
            Input::Create(create) => create_answer(create),
            Input::CreateAndRun(run) => create_and_run_answer(run),
            Input::Delete(delete) => delete_answer(delete),
        }
    }
}

fn build_dispatch(http: &FakeHttp, environment: Environment) -> Dispatch {
    SandboxDispatch {
        e2b: E2bSandbox {
            http: http.clone(),
            environment: environment.clone(),
        },
        opensandbox: OpenSandbox {
            http: http.clone(),
            environment,
            clock: StepClock::default(),
        },
    }
}

fn registry_answer(input: &RegistryInput) -> HelperResult {
    let entries = input
        .tools
        .iter()
        .map(|tool| {
            let params = tool.litellm_params.as_ref();
            SandboxToolEntry::Configured(SandboxToolConfig {
                name: tool.sandbox_tool_name.clone(),
                provider: params.and_then(|params| params.sandbox_provider.clone()),
                api_key: params.and_then(|params| params.api_key.clone()),
                api_base: params.and_then(|params| params.api_base.clone()),
            })
        })
        .collect::<Vec<_>>();
    let mut registry = SandboxToolRegistry::default();
    registry
        .register(&entries, &secrets(), &input.environment)
        .map_err(|error| helper_error("SandboxRegistryError", error))?;
    let mut resolved = serde_json::Map::new();
    for name in &input.names {
        let tool = registry.resolve(name).map(|tool| {
            serde_json::json!({
                "sandbox_provider": tool.provider,
                "api_key": tool.api_key.as_ref().map(|key| key.expose_secret()),
                "api_base": tool.api_base,
            })
        });
        resolved.insert(name.clone(), tool.unwrap_or(Value::Null));
    }
    Ok(serde_json::json!({"resolved": resolved}))
}

fn create_answer(input: &CreateInput) -> HelperResult {
    let http = FakeHttp::new(&input.replies);
    let sandbox = build_dispatch(&http, input.environment.clone());
    let api_key = input.api_key.as_deref().map(SecretString::from);
    let kwargs = IndexMap::new();
    let policy = raw_policy(input)?;
    let mut request = create_request(input, &api_key, &kwargs, policy.as_deref())?;
    let created = block_on(sandbox.acreate_sandbox(&mut request))?;
    Ok(serde_json::json!({
        "requests": http.requests(),
        "handle": container_value(&created),
    }))
}

fn create_and_run_answer(input: &RunInput) -> HelperResult {
    let http = FakeHttp::new(&input.create.replies);
    let sandbox = build_dispatch(&http, input.create.environment.clone());
    let api_key = input.create.api_key.as_deref().map(SecretString::from);
    let kwargs = IndexMap::new();
    let policy = raw_policy(&input.create)?;
    let mut create_call = create_request(&input.create, &api_key, &kwargs, policy.as_deref())?;
    let created = block_on(sandbox.acreate_sandbox(&mut create_call))?;
    let mut run = RunCode {
        call: SandboxCall {
            provider: &input.create.provider,
            api_key: api_key.as_ref(),
            kwargs: &kwargs,
            logging: None,
        },
        container: SandboxContainerRef::Container(&created),
        code: &input.code,
        api_base: input.create.api_base.as_deref(),
        env_vars: input.create.env_vars.as_ref(),
        language: input.language.as_deref(),
        use_server_proxy: input.create.use_server_proxy.unwrap_or(false),
        ready_timeout: duration(input.create.ready_timeout)?,
        poll_interval: duration(input.create.poll_interval)?,
    };
    let result = block_on(sandbox.arun_code(&mut run))?;
    Ok(serde_json::json!({
        "requests": http.requests(),
        "handle": container_value(&created),
        "result": result,
    }))
}

fn delete_answer(input: &DeleteInput) -> HelperResult {
    let http = FakeHttp::new(&input.replies);
    let sandbox = build_dispatch(&http, input.environment.clone());
    let api_key = input.api_key.as_deref().map(SecretString::from);
    let kwargs = IndexMap::new();
    let mut request = DeleteSandbox {
        call: SandboxCall {
            provider: &input.provider,
            api_key: api_key.as_ref(),
            kwargs: &kwargs,
            logging: None,
        },
        container: SandboxContainerRef::Id(&input.container),
        api_base: input.api_base.as_deref(),
    };
    let deleted = block_on(sandbox.adelete_sandbox(&mut request))?;
    Ok(serde_json::json!({"requests": http.requests(), "deleted": deleted}))
}

fn create_request<'call>(
    input: &'call CreateInput,
    api_key: &'call Option<SecretString>,
    kwargs: &'call IndexMap<String, Json>,
    network_policy: Option<&'call serde_json::value::RawValue>,
) -> Result<CreateSandbox<'call>, Box<HelperError>> {
    Ok(CreateSandbox {
        call: SandboxCall {
            provider: &input.provider,
            api_key: api_key.as_ref(),
            kwargs,
            logging: None,
        },
        api_base: input.api_base.as_deref(),
        template: input.template.as_deref(),
        timeout: input.timeout,
        allow_internet_access: input.allow_internet_access,
        metadata: input.metadata.as_ref(),
        env_vars: input.env_vars.as_ref(),
        resource_limits: input.resource_limits.as_ref(),
        resource_requests: input.resource_requests.as_ref(),
        entrypoint: input.entrypoint.as_deref(),
        network_policy,
        secure_access: input.secure_access.unwrap_or(false),
        use_server_proxy: input.use_server_proxy.unwrap_or(false),
        ready_timeout: duration(input.ready_timeout)?,
        poll_interval: duration(input.poll_interval)?,
    })
}

fn raw_policy(
    input: &CreateInput,
) -> Result<Option<Box<serde_json::value::RawValue>>, Box<HelperError>> {
    input
        .network_policy
        .as_ref()
        .map(|policy| {
            serde_json::value::to_raw_value(policy)
                .map_err(|error| helper_error("ValueError", error))
        })
        .transpose()
}

fn duration(seconds: Option<f64>) -> Result<Option<Duration>, Box<HelperError>> {
    seconds
        .map(|value| {
            Duration::try_from_secs_f64(value).map_err(|error| helper_error("ValueError", error))
        })
        .transpose()
}

fn block_on<Output, Failure, Work>(work: Work) -> Result<Output, Box<HelperError>>
where
    Work: std::future::Future<Output = Result<Output, Failure>>,
    Failure: std::fmt::Display,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error))?;
    runtime
        .block_on(work)
        .map_err(|error| helper_error("SandboxError", error))
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
