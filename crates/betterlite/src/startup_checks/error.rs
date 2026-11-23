use betterlite_guardrails::prompt_injection::error::PromptInjectionError;
use thiserror::Error;

use crate::startup_checks::team_budgets::TeamEntryProblem;

#[derive(Debug, Error)]
pub enum StartupCheckError {
    #[error(
        "`use_redis_transaction_buffer` is enabled in general_settings but no Redis is configured. This will cause spend updates to not be tracked. Add a Redis cache in litellm_settings:\n\nlitellm_settings:\n  cache: true\n  cache_params:\n    type: redis\n    url: os.environ/REDIS_URL\n\nor set REDIS_* environment variables (e.g. REDIS_HOST, REDIS_PORT, REDIS_PASSWORD, or REDIS_URL) to use a standalone Redis for the transaction buffer."
    )]
    TransactionBuffer,
    #[error("budget_duration not set on Proxy. budget_duration is required to use max_budget.")]
    BudgetDurationNotSet,
    #[error("litellm_settings.default_internal_user_params.teams must be a list of objects")]
    TeamsNotAList,
    #[error(transparent)]
    PromptInjection(#[from] PromptInjectionError),
    #[error(transparent)]
    TeamEntry(#[from] TeamEntryProblem),
    #[error("model_list: {violation} {remedy}")]
    TuningLimit {
        violation: String,
        remedy: &'static str,
    },
}
