use betterlite_config::sections::mock_testing_params;
use betterlite_logging::analytics::python::float_text;
use betterlite_server::messages_call::steps::MOCK_TESTING_CONFIG_KEY;
use tracing::warn;

use crate::startup_checks::presence::{DatabasePresence, RedisPresence};

// litellm.max_budget is a float in LiteLLM, so the message carries Python's repr of it
pub fn warn_budget_without_db(max_budget: Option<f64>, database: DatabasePresence) {
    let Some(max_budget) = max_budget else {
        return;
    };
    if database == DatabasePresence::Present || max_budget <= 0.0_f64 {
        return;
    }
    let budget = float_text(max_budget);
    warn!(
        "A proxy-wide budget (litellm.max_budget={budget}) is configured but no database is connected, so the budget will NOT be enforced and requests will never be blocked. Set DATABASE_URL or general_settings.database_url and restart. Redis and fail_closed_budget_enforcement do not cover the proxy-wide budget because there is no global spend counter; Redis alone is not a substitute."
    );
}

pub fn warn_fail_closed_without_redis(enabled: bool, redis: RedisPresence) {
    if !enabled || redis == RedisPresence::Present {
        return;
    }
    warn!(
        "general_settings.fail_closed_rate_limit_enforcement is enabled but no Redis is configured, so rate limits are enforced per pod from memory and the setting rejects nothing. Configure general_settings.coordination_redis (or REDIS_HOST/REDIS_PORT/REDIS_PASSWORD) to share the counters across pods and make the setting effective."
    );
}

pub fn warn_mock_testing(settings: &mock_testing_params::Settings, gated_names: &[&str]) {
    if !settings.allowed {
        return;
    }
    let banner = "=".repeat(72);
    let names = gated_names
        .iter()
        .map(|name| format!("   {name}"))
        .collect::<Vec<_>>()
        .join("\n");
    let message = format!(
        "\n{banner}\n DANGEROUS SETTING ENABLED\n general_settings.{MOCK_TESTING_CONFIG_KEY} = true\n\n Any caller with a valid key on this proxy can now inject synthetic\n failures and latency into their own requests using these body params:\n{names}\n\n A request using them consumes a connection and a concurrency slot\n without reaching a provider, and returns an error the caller chose.\n\n Intended for testing fallback chains. Do not leave enabled.\n{banner}"
    );
    warn!("{message}");
}
