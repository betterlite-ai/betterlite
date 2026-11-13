use std::time::Duration;

pub const LITELLM_SKILL_ID_PREFIX: &str = "litellm_skill_";
pub const DEFAULT_MAX_ITERATIONS: u32 = 10;
pub const DEFAULT_SANDBOX_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HookKwargs {
    pub max_iterations: Option<u32>,
    pub sandbox_timeout: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookSettings {
    pub max_iterations: u32,
    pub sandbox_timeout: Duration,
}

impl From<HookKwargs> for HookSettings {
    fn from(kwargs: HookKwargs) -> Self {
        Self {
            max_iterations: kwargs.max_iterations.unwrap_or(DEFAULT_MAX_ITERATIONS),
            sandbox_timeout: kwargs.sandbox_timeout.unwrap_or(DEFAULT_SANDBOX_TIMEOUT),
        }
    }
}
