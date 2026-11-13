use std::time::Duration;

pub const PRISMA_COMMAND_TIMEOUT_ENV_VAR: &str = "LITELLM_PRISMA_COMMAND_TIMEOUT";
pub const PRISMA_DEPLOY_TIMEOUT_ENV_VAR: &str = "LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT";
pub const MIGRATION_LOCK_TIMEOUT_ENV_VAR: &str = "LITELLM_MIGRATION_LOCK_TIMEOUT";
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
pub const DEFAULT_DEPLOY_TIMEOUT: Duration = Duration::from_secs(600);
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(600);

// Mirrors prisma_toolchain.py::_timeout_from_env: an unset variable is silent, and unusable text warns before the default applies.
pub fn timeout_from_env(name: &str, raw: Option<&str>, default: Duration) -> Duration {
    let Some(text) = raw else {
        return default;
    };
    let fallback = default.as_secs();
    let Ok(seconds) = text.trim().parse::<f64>() else {
        tracing::warn!(
            environment_variable = %name,
            value = %text,
            fallback_seconds = fallback,
            "{name} is not a number, falling back to {fallback}s"
        );
        return default;
    };
    if !seconds.is_finite() || seconds <= 0.0_f64 {
        tracing::warn!(
            environment_variable = %name,
            value = %text,
            fallback_seconds = fallback,
            "{name} is not a finite positive number, falling back to {fallback}s"
        );
        return default;
    }
    Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX)
}

// Deploy inherits the command budget only when its own variable is absent, never when it is present but unusable.
#[must_use]
pub fn prisma_migrate_deploy_timeout(deploy: Option<&str>, command: Option<&str>) -> Duration {
    deploy.map_or_else(
        || {
            DEFAULT_DEPLOY_TIMEOUT.max(timeout_from_env(
                PRISMA_COMMAND_TIMEOUT_ENV_VAR,
                command,
                DEFAULT_COMMAND_TIMEOUT,
            ))
        },
        |text| {
            timeout_from_env(
                PRISMA_DEPLOY_TIMEOUT_ENV_VAR,
                Some(text),
                DEFAULT_DEPLOY_TIMEOUT,
            )
        },
    )
}
