use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::sandbox::CodeExecutionResult;

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

// the two adapters return different handles and take different options, so the dispatch drives them through one seam
#[async_trait]
pub trait SandboxAdapter: Send + Sync {
    async fn create(
        &self,
        request: &CreateSandbox<'_>,
    ) -> Result<SandboxContainer, SandboxLifecycleError>;

    async fn run(
        &self,
        request: &RunCode<'_>,
    ) -> Result<CodeExecutionResult, SandboxLifecycleError>;

    async fn delete(&self, request: &DeleteSandbox<'_>) -> Result<bool, SandboxLifecycleError>;
}

#[async_trait]
impl<Http, Environment> SandboxAdapter for E2bSandbox<Http, Environment>
where
    Http: crate::e2b::SandboxHttp,
    Environment: EnvironmentLookup + Send + Sync,
{
    async fn create(
        &self,
        request: &CreateSandbox<'_>,
    ) -> Result<SandboxContainer, SandboxLifecycleError> {
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

    async fn run(
        &self,
        request: &RunCode<'_>,
    ) -> Result<CodeExecutionResult, SandboxLifecycleError> {
        let container = match request.container {
            SandboxContainerRef::Container(SandboxContainer::E2b(handle)) => handle,
            SandboxContainerRef::Container(SandboxContainer::OpenSandbox(handle)) => {
                &handle.container
            }
            SandboxContainerRef::Id(_) => {
                return Err(SandboxLifecycleError::ContainerHandleRequired { provider: "e2b" });
            }
        };
        self.arun_code(E2bRun {
            container,
            code: request.code,
            env_vars: request.env_vars,
        })
        .await
        .map_err(SandboxLifecycleError::E2b)
    }

    async fn delete(&self, request: &DeleteSandbox<'_>) -> Result<bool, SandboxLifecycleError> {
        let container = match request.container {
            SandboxContainerRef::Id(id) => SandboxRef::Id(id),
            SandboxContainerRef::Container(SandboxContainer::E2b(handle)) => {
                SandboxRef::Handle(handle)
            }
            SandboxContainerRef::Container(SandboxContainer::OpenSandbox(handle)) => {
                SandboxRef::Handle(&handle.container)
            }
        };
        self.adelete_sandbox(E2bDelete {
            container,
            api_key: request.call.api_key,
            api_base: request.api_base,
        })
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
    async fn create(
        &self,
        request: &CreateSandbox<'_>,
    ) -> Result<SandboxContainer, SandboxLifecycleError> {
        let handle = self
            .acreate_sandbox(
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
                },
                None,
            )
            .await
            .map_err(SandboxLifecycleError::OpenSandbox)?;
        Ok(SandboxContainer::OpenSandbox(handle))
    }

    async fn run(
        &self,
        request: &RunCode<'_>,
    ) -> Result<CodeExecutionResult, SandboxLifecycleError> {
        let container = match request.container {
            SandboxContainerRef::Container(SandboxContainer::OpenSandbox(handle)) => {
                OpenContainerRef::Cached(handle)
            }
            SandboxContainerRef::Container(SandboxContainer::E2b(handle)) => {
                OpenContainerRef::Handle(handle)
            }
            SandboxContainerRef::Id(id) => OpenContainerRef::Id(id),
        };
        self.arun_code(
            OpenRun {
                container,
                code: request.code,
                language: request.language,
                options: OpenOptions {
                    api_key: request.call.api_key,
                    api_base: request.api_base,
                    use_server_proxy: request.use_server_proxy,
                    ready_timeout: request.ready_timeout,
                    poll_interval: request.poll_interval,
                },
            },
            None,
        )
        .await
        .map_err(SandboxLifecycleError::OpenSandbox)
    }

    async fn delete(&self, request: &DeleteSandbox<'_>) -> Result<bool, SandboxLifecycleError> {
        let container = match request.container {
            SandboxContainerRef::Container(SandboxContainer::OpenSandbox(handle)) => {
                OpenContainerRef::Cached(handle)
            }
            SandboxContainerRef::Container(SandboxContainer::E2b(handle)) => {
                OpenContainerRef::Handle(handle)
            }
            SandboxContainerRef::Id(id) => OpenContainerRef::Id(id),
        };
        self.adelete_sandbox(
            OpenDelete {
                container,
                options: OpenOptions {
                    api_key: request.call.api_key,
                    api_base: request.api_base,
                    use_server_proxy: false,
                    ready_timeout: None,
                    poll_interval: None,
                },
            },
            None,
        )
        .await
        .map_err(SandboxLifecycleError::OpenSandbox)
    }
}
