use betterlite_models::sandbox::CodeExecutionResult;

use super::adapter::SandboxAdapter;
use super::error::SandboxLifecycleError;
use super::factory::SandboxProvider;
use super::logging::update_logging;
use super::request::{CreateSandbox, DeleteSandbox, RunCode, SandboxContainer};

mod tests;

pub struct SandboxDispatch<E2b, OpenSandbox> {
    pub e2b: E2b,
    pub opensandbox: OpenSandbox,
}

impl<E2b, OpenSandbox> SandboxDispatch<E2b, OpenSandbox>
where
    E2b: SandboxAdapter,
    OpenSandbox: SandboxAdapter,
{
    pub async fn acreate_sandbox(
        &self,
        request: &mut CreateSandbox<'_>,
    ) -> Result<SandboxContainer, SandboxLifecycleError> {
        update_logging(&mut request.call, "create_sandbox")?;
        match SandboxProvider::from_name(request.call.provider)? {
            SandboxProvider::E2b => self.e2b.create(request).await,
            SandboxProvider::OpenSandbox => self.opensandbox.create(request).await,
        }
    }

    pub async fn arun_code(
        &self,
        request: &mut RunCode<'_>,
    ) -> Result<CodeExecutionResult, SandboxLifecycleError> {
        update_logging(&mut request.call, "run_code")?;
        match SandboxProvider::from_name(request.call.provider)? {
            SandboxProvider::E2b => self.e2b.run(request).await,
            SandboxProvider::OpenSandbox => self.opensandbox.run(request).await,
        }
    }

    pub async fn adelete_sandbox(
        &self,
        request: &mut DeleteSandbox<'_>,
    ) -> Result<bool, SandboxLifecycleError> {
        update_logging(&mut request.call, "delete_sandbox")?;
        match SandboxProvider::from_name(request.call.provider)? {
            SandboxProvider::E2b => self.e2b.delete(request).await,
            SandboxProvider::OpenSandbox => self.opensandbox.delete(request).await,
        }
    }
}
