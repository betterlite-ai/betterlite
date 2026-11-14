use super::error::SandboxLifecycleError;

// LiteLLM's SandboxProviders enum names the closed set the factory accepts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxProvider {
    E2b,
    OpenSandbox,
}

impl SandboxProvider {
    pub fn from_name(provider: &str) -> Result<Self, SandboxLifecycleError> {
        match provider {
            "e2b" => Ok(Self::E2b),
            "opensandbox" => Ok(Self::OpenSandbox),
            other => Err(SandboxLifecycleError::UnsupportedProvider {
                provider: other.to_owned(),
            }),
        }
    }
}
