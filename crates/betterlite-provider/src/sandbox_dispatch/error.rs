use betterlite_logging::logging_state::LoggingStateError;
use thiserror::Error;

use crate::e2b::E2bError;
use crate::opensandbox::lifecycle::LifecycleError;

#[derive(Debug, Error)]
pub enum SandboxLifecycleError {
    // LiteLLM builds the message from the provider string the same way, in its _get_config
    #[error("code execution is not supported for provider: {provider}")]
    UnsupportedProvider { provider: String },
    #[error("run_code for provider {provider} needs the container handle acreate_sandbox returned")]
    ContainerHandleRequired { provider: &'static str },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    E2b(#[from] E2bError),
    #[error(transparent)]
    OpenSandbox(#[from] LifecycleError),
}
