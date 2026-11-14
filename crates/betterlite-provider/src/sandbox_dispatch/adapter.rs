use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::sandbox::{CodeExecutionResult, ContainerHandle};

use crate::e2b::{
    CreateSandbox as E2bCreate, DeleteSandbox as E2bDelete, E2bSandbox, RunCode as E2bRun,
    SandboxRef,
};
use crate::opensandbox::helpers::{OpenSandbox, PollClock};
use crate::opensandbox::lifecycle::{
    ContainerRef as OpenContainerRef, CreateSandboxRequest as OpenCreate,
    DeleteSandbox as OpenDelete, RunCode as OpenRun, SandboxOptions as OpenOptions,
};

use super::error::SandboxLifecycleError;
use super::request::{
    CreateSandbox, DeleteSandbox, RunCode, SandboxContainer, SandboxContainerRef,
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
    Http: crate::e2b::SandboxHttp,
    Environment: EnvironmentLookup + Send + Sync,
{
    async fn create(&self, request: &CreateSandbox<'_>) -> AdapterResult<SandboxContainer> {
        let created = self
            .acreate_sandbox(e2b_create(request))
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
    Http: crate::opensandbox::helpers::SandboxHttp,
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
        self.arun_code(open_run(request), None)
            .await
            .map_err(SandboxLifecycleError::OpenSandbox)
    }

    async fn delete(&self, request: &DeleteSandbox<'_>) -> AdapterResult<bool> {
        self.adelete_sandbox(open_delete(request), None)
            .await
            .map_err(SandboxLifecycleError::OpenSandbox)
    }
}

fn e2b_create(request: &CreateSandbox<'_>) -> E2bCreate<'_> {
    E2bCreate {
        template: request.template,
        timeout: request.timeout,
        allow_internet_access: request.allow_internet_access,
        api_key: request.call.api_key,
        api_base: request.api_base,
    }
}

fn e2b_run(request: &RunCode<'_>) -> AdapterResult<E2bRun<'_>> {
    Ok(E2bRun {
        container: e2b_container(request.container)?,
        code: request.code,
        env_vars: request.env_vars,
    })
}

fn e2b_delete(request: &DeleteSandbox<'_>) -> E2bDelete<'_> {
    E2bDelete {
        container: e2b_ref(request.container),
        api_key: request.call.api_key,
        api_base: request.api_base,
    }
}

fn e2b_container(container: SandboxContainerRef<'_>) -> AdapterResult<&ContainerHandle> {
    let SandboxContainerRef::Container(sandbox) = container else {
        return Err(SandboxLifecycleError::ContainerHandleRequired { provider: "e2b" });
    };
    match sandbox {
        SandboxContainer::E2b(handle) => Ok(handle),
        SandboxContainer::OpenSandbox(handle) => Ok(&handle.container),
    }
}

fn e2b_ref(container: SandboxContainerRef<'_>) -> SandboxRef<'_> {
    match container {
        SandboxContainerRef::Id(id) => SandboxRef::Id(id),
        SandboxContainerRef::Container(SandboxContainer::E2b(handle)) => SandboxRef::Handle(handle),
        SandboxContainerRef::Container(SandboxContainer::OpenSandbox(handle)) => {
            SandboxRef::Handle(&handle.container)
        }
    }
}

fn open_create(request: &CreateSandbox<'_>) -> OpenCreate<'_> {
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
        options: OpenOptions {
            api_key: request.call.api_key,
            api_base: request.api_base,
            use_server_proxy: request.use_server_proxy,
            ready_timeout: request.ready_timeout,
            poll_interval: request.poll_interval,
        },
    }
}

fn open_run(request: &RunCode<'_>) -> OpenRun<'_> {
    OpenRun {
        container: open_container(request.container),
        code: request.code,
        language: request.language,
        options: OpenOptions {
            api_key: request.call.api_key,
            api_base: request.api_base,
            use_server_proxy: request.use_server_proxy,
            ready_timeout: request.ready_timeout,
            poll_interval: request.poll_interval,
        },
    }
}

fn open_delete(request: &DeleteSandbox<'_>) -> OpenDelete<'_> {
    let options = OpenOptions {
        api_key: request.call.api_key,
        api_base: request.api_base,
        ..OpenOptions::default()
    };
    OpenDelete {
        container: open_container(request.container),
        options,
    }
}

fn open_container(container: SandboxContainerRef<'_>) -> OpenContainerRef<'_> {
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
