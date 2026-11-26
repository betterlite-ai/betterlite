use betterlite_provider::sandbox_dispatch::request::{
    RunCode, SandboxContainer, SandboxContainerRef,
};

use super::super::convert::{Arguments, ConvertError, requested_timeout};
use super::sandbox_call;

// The run request executes inside the container acreate_sandbox returned.
pub(crate) fn run_request<'call>(
    arguments: &'call Arguments,
    container: &'call SandboxContainer,
) -> Result<RunCode<'call>, ConvertError> {
    let call = sandbox_call(arguments);
    Ok(RunCode {
        api_base: arguments.api_base.as_deref(),
        call,
        code: &arguments.code,
        container: SandboxContainerRef::Container(container),
        env_vars: arguments.env_vars.as_ref(),
        language: arguments.language.as_deref(),
        poll_interval: requested_timeout(arguments.poll_interval)?,
        ready_timeout: requested_timeout(arguments.ready_timeout)?,
        use_server_proxy: arguments.use_server_proxy.unwrap_or_default(),
    })
}
