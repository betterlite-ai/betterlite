use betterlite_models::sandbox::CodeExecutionResult;

use super::adapter::SandboxAdapter;
use super::error::SandboxLifecycleError;
use super::factory::SandboxProvider;
use super::logging::update_logging;
use super::request::{CreateSandbox, DeleteSandbox, RunCode, SandboxCall, SandboxContainer};

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
        self.adapter(&mut request.call, "create_sandbox")?
            .create(request)
            .await
    }

    pub async fn arun_code(
        &self,
        request: &mut RunCode<'_>,
    ) -> Result<CodeExecutionResult, SandboxLifecycleError> {
        self.adapter(&mut request.call, "run_code")?
            .run(request)
            .await
    }

    pub async fn adelete_sandbox(
        &self,
        request: &mut DeleteSandbox<'_>,
    ) -> Result<bool, SandboxLifecycleError> {
        self.adapter(&mut request.call, "delete_sandbox")?
            .delete(request)
            .await
    }

    fn adapter<'adapter>(
        &'adapter self,
        call: &mut SandboxCall<'_>,
        operation: &str,
    ) -> Result<&'adapter dyn SandboxAdapter, SandboxLifecycleError> {
        update_logging(call, operation)?;
        let provider = SandboxProvider::from_name(call.provider)?;
        Ok(provider.select(&self.e2b, &self.opensandbox))
    }
}
