use crate::sandbox_lifecycle::cache_operations::SandboxCache;
use crate::sandbox_lifecycle::lifecycle::{SandboxConfig, SandboxProvider, SandboxRegistry};
use crate::sandbox_lifecycle::resolution::SandboxToolRegistry;

use super::error::PlanError;

// LiteLLM deletes the cached container on any plan-building failure and hands the failure back
#[must_use]
pub async fn delete_after_failure<C, R, T>(
    provider: &SandboxProvider<'_, C, R, T>,
    cache: &mut SandboxCache,
    sandbox_key: Option<&str>,
    failure: PlanError<C::Error>,
) -> PlanError<C::Error>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    provider
        .delete_container_for_cache_key(cache, sandbox_key)
        .await;
    failure
}

include!("error_handling/tests/cases.rs");
