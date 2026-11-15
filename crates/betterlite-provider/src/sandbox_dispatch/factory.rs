use super::adapter::SandboxAdapter;
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

    #[must_use]
    pub fn select<'adapter, E2b, OpenSandbox>(
        self,
        e2b: &'adapter E2b,
        opensandbox: &'adapter OpenSandbox,
    ) -> &'adapter dyn SandboxAdapter
    where
        E2b: SandboxAdapter,
        OpenSandbox: SandboxAdapter,
    {
        match self {
            Self::E2b => e2b,
            Self::OpenSandbox => opensandbox,
        }
    }
}
