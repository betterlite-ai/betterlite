use betterlite_config::loader::EnvironmentLookup;

pub const RECORD_CONTENT_ENV: &str = "NEW_RELIC_AI_MONITORING_RECORD_CONTENT_ENABLED";

#[derive(Clone, Copy)]
pub struct ContentPolicy<'env> {
    pub turn_off_message_logging: bool,
    pub redact_message_logging: bool,
    pub environment: &'env dyn EnvironmentLookup,
}

impl ContentPolicy<'_> {
    #[must_use]
    pub fn record_content(&self) -> bool {
        if self.turn_off_message_logging {
            return false;
        }
        content_enabled_by_env(self.environment)
    }
}

// LiteLLM's default is true, and an empty variable counts as unset.
pub fn content_enabled_by_env(environment: &dyn EnvironmentLookup) -> bool {
    let Some(raw) = environment.lookup(RECORD_CONTENT_ENV) else {
        return true;
    };
    match betterlite_env::flag(RECORD_CONTENT_ENV, &raw) {
        Ok(Some(enabled)) => enabled,
        Ok(None) => true,
        Err(_source) => {
            tracing::warn!(
                variable = RECORD_CONTENT_ENV,
                "not a recognised boolean; recording content"
            );
            true
        }
    }
}
