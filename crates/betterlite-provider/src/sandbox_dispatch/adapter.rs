use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::sandbox::CodeExecutionResult;

use crate::e2b::{
    CreateSandbox as E2bCreate, DeleteSandbox as E2bDelete, E2bSandbox, RunCode as E2bRun,
    SandboxHttp as E2bHttp, SandboxRef,
};
use crate::opensandbox::helpers::{OpenSandbox, PollClock, SandboxHttp as OpenSandboxHttp};
use crate::opensandbox::lifecycle::{
    ContainerRef as OpenContainerRef, CreateSandboxRequest as OpenCreate,
    DeleteSandbox as OpenDelete, RunCode as OpenRun, SandboxOptions as OpenOptions,
};

use super::error::SandboxLifecycleError;
use super::request::{
    CreateSandbox, DeleteSandbox, RunCode, SandboxCall, SandboxContainer, SandboxContainerRef,
};

type AdapterResult<Value> = Result<Value, SandboxLifecycleError>;

// the two adapters return different handles and take different options, so the dispatch drives them through one seam
#[async_trait]
pub trait SandboxAdapter: Send + Sync {
    async fn create(&self, request: &CreateSandbox<'_>) -> AdapterResult<SandboxContainer>;

    async fn run(&self, request: &RunCode<'_>) -> AdapterResult<CodeExecutionResult>;

    async fn delete(&self, request: &DeleteSandbox<'_>) -> AdapterResult<bool>;
}

#[async_trait]
impl<Http, Environment> SandboxAdapter for E2bSandbox<Http, Environment>
where
    Http: E2bHttp,
    Environment: EnvironmentLookup + Send + Sync,
{
    async fn create(&self, request: &CreateSandbox<'_>) -> AdapterResult<SandboxContainer> {
        let created = self
            .acreate_sandbox(E2bCreate {
                template: request.template,
                timeout: request.timeout,
                allow_internet_access: request.allow_internet_access,
                api_key: request.call.api_key,
                api_base: request.api_base,
            })
            .await
            .map_err(SandboxLifecycleError::E2b)?;
        Ok(SandboxContainer::E2b(created))
    }

    async fn run(&self, request: &RunCode<'_>) -> AdapterResult<CodeExecutionResult> {
        self.arun_code(e2b_run(request)?)
            .await
            .map_err(SandboxLifecycleError::E2b)
    }

    async fn delete(&self, request: &DeleteSandbox<'_>) -> AdapterResult<bool> {
        self.adelete_sandbox(e2b_delete(request))
            .await
            .map_err(SandboxLifecycleError::E2b)
    }
}

#[async_trait]
impl<Http, Environment, Clock> SandboxAdapter for OpenSandbox<Http, Environment, Clock>
where
    Http: OpenSandboxHttp,
    Environment: EnvironmentLookup + Send + Sync,
    Clock: PollClock,
{
    async fn create(&self, request: &CreateSandbox<'_>) -> AdapterResult<SandboxContainer> {
        let handle = self
            .acreate_sandbox(open_create(request), None)
            .await
            .map_err(SandboxLifecycleError::OpenSandbox)?;
        Ok(SandboxContainer::OpenSandbox(handle))
    }

    async fn run(&self, request: &RunCode<'_>) -> AdapterResult<CodeExecutionResult> {
        let container = open_container(request.container);
        let options = open_options(
            &request.call,
            request.api_base,
            request.use_server_proxy,
            request.ready_timeout,
            request.poll_interval,
        );
        self.arun_code(
            OpenRun {
                container,
                code: request.code,
                language: request.language,
                options,
            },
            None,
        )
        .await
        .map_err(SandboxLifecycleError::OpenSandbox)
    }

    async fn delete(&self, request: &DeleteSandbox<'_>) -> AdapterResult<bool> {
        self.adelete_sandbox(open_delete(request), None)
            .await
            .map_err(SandboxLifecycleError::OpenSandbox)
    }
}

const fn e2b_run<'call>(request: &RunCode<'call>) -> AdapterResult<E2bRun<'call>> {
    let SandboxContainerRef::Container(sandbox) = request.container else {
        return Err(SandboxLifecycleError::ContainerHandleRequired { provider: "e2b" });
    };
    let container = match sandbox {
        SandboxContainer::E2b(handle) => handle,
        SandboxContainer::OpenSandbox(handle) => &handle.container,
    };
    Ok(E2bRun {
        container,
        code: request.code,
        env_vars: request.env_vars,
    })
}

const fn e2b_delete<'call>(request: &DeleteSandbox<'call>) -> E2bDelete<'call> {
    E2bDelete {
        container: e2b_ref(request.container),
        api_key: request.call.api_key,
        api_base: request.api_base,
    }
}

const fn e2b_ref(container: SandboxContainerRef<'_>) -> SandboxRef<'_> {
    match open_container(container) {
        OpenContainerRef::Id(id) => SandboxRef::Id(id),
        OpenContainerRef::Handle(handle) => SandboxRef::Handle(handle),
        OpenContainerRef::Cached(handle) => SandboxRef::Handle(&handle.container),
    }
}

const fn open_create<'call>(request: &CreateSandbox<'call>) -> OpenCreate<'call> {
    OpenCreate {
        template: request.template,
        timeout: request.timeout,
        allow_internet_access: request.allow_internet_access,
        metadata: request.metadata,
        env_vars: request.env_vars,
        resource_limits: request.resource_limits,
        resource_requests: request.resource_requests,
        entrypoint: request.entrypoint,
        network_policy: request.network_policy,
        secure_access: request.secure_access,
        options: open_options(
            &request.call,
            request.api_base,
            request.use_server_proxy,
            request.ready_timeout,
            request.poll_interval,
        ),
    }
}

const fn open_delete<'call>(request: &DeleteSandbox<'call>) -> OpenDelete<'call> {
    OpenDelete {
        container: open_container(request.container),
        options: open_options(&request.call, request.api_base, false, None, None),
    }
}

const fn open_options<'call>(
    call: &SandboxCall<'call>,
    api_base: Option<&'call str>,
    use_server_proxy: bool,
    ready_timeout: Option<Duration>,
    poll_interval: Option<Duration>,
) -> OpenOptions<'call> {
    OpenOptions {
        api_key: call.api_key,
        api_base,
        use_server_proxy,
        ready_timeout,
        poll_interval,
    }
}

const fn open_container(container: SandboxContainerRef<'_>) -> OpenContainerRef<'_> {
    match container {
        SandboxContainerRef::Container(SandboxContainer::OpenSandbox(handle)) => {
            OpenContainerRef::Cached(handle)
        }
        SandboxContainerRef::Container(SandboxContainer::E2b(handle)) => {
            OpenContainerRef::Handle(handle)
        }
        SandboxContainerRef::Id(id) => OpenContainerRef::Id(id),
    }
}
