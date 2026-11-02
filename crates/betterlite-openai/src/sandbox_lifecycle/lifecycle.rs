use std::error::Error as StdError;
use std::future::Future;

use serde_json::Value;
use thiserror::Error;

use super::cache_operations::SandboxCache;
use super::resolution::{self, SandboxToolParams, SandboxToolRegistry};

pub trait SandboxConfig: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn create(&self) -> impl Future<Output = Result<Value, Self::Error>> + Send;
    fn run(
        &self,
        container: &Value,
        code: &str,
    ) -> impl Future<Output = Result<Value, Self::Error>> + Send;
    fn delete(&self, container: &Value) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

pub trait SandboxRegistry: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn create(
        &self,
        params: &SandboxToolParams,
    ) -> impl Future<Output = Result<Value, Self::Error>> + Send;
    fn run(
        &self,
        params: &SandboxToolParams,
        container: &Value,
        code: &str,
    ) -> impl Future<Output = Result<Value, Self::Error>> + Send;
    fn delete(
        &self,
        params: &SandboxToolParams,
        container: &Value,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[derive(Debug, Error)]
pub enum LifecycleError<E>
where
    E: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Provider(E),
    #[error("CodeInterpreterInterception: no sandbox available")]
    NoSandboxAvailable,
}

pub struct SandboxProvider<'provider, C, R, T>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    pub sandbox_config: Option<&'provider C>,
    pub registry: &'provider R,
    pub tool_registry: &'provider T,
    pub sandbox_tool_name: Option<&'provider str>,
}

impl<C, R, T> SandboxProvider<'_, C, R, T>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    pub async fn create_container(
        &self,
    ) -> Result<(Value, Option<SandboxToolParams>), LifecycleError<C::Error>> {
        if let Some(config) = self.sandbox_config {
            let container = config.create().await.map_err(LifecycleError::Provider)?;
            return Ok((container, None));
        }
        let params = resolution::resolve_sandbox_tool(self.sandbox_tool_name, self.tool_registry)
            .ok_or(LifecycleError::NoSandboxAvailable)?;
        let container = self
            .registry
            .create(&params)
            .await
            .map_err(LifecycleError::Provider)?;
        Ok((container, Some(params)))
    }

    pub async fn run_code(
        &self,
        params: Option<&SandboxToolParams>,
        container: &Value,
        code: &str,
    ) -> Result<Value, LifecycleError<C::Error>> {
        if let Some(config) = self.sandbox_config {
            return config
                .run(container, code)
                .await
                .map_err(LifecycleError::Provider);
        }
        let resolved_params = params.ok_or(LifecycleError::NoSandboxAvailable)?;
        self.registry
            .run(resolved_params, container, code)
            .await
            .map_err(LifecycleError::Provider)
    }

    pub async fn delete_container(&self, params: Option<&SandboxToolParams>, container: &Value) {
        let outcome = match self.sandbox_config {
            Some(config) => config
                .delete(container)
                .await
                .map_err(LifecycleError::Provider),
            None => match params {
                Some(registry_params) => self
                    .registry
                    .delete(registry_params, container)
                    .await
                    .map_err(LifecycleError::Provider),
                None => return,
            },
        };
        if let Err(error) = outcome {
            tracing::error!(
                "CodeInterpreterInterception: failed to delete sandbox container: {error}"
            );
        }
    }

    pub async fn delete_container_for_cache_key(
        &self,
        cache: &mut SandboxCache,
        cache_key: Option<&str>,
    ) {
        let Some(key) = cache_key else { return };
        let Some(entry) = cache.entries.remove(key) else {
            return;
        };
        self.delete_container(entry.params.as_ref(), &entry.container)
            .await;
    }
}

include!("lifecycle/tests/cases.rs");
