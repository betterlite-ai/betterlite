use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::api::summary_execution::error::SummaryExecutionError;
use crate::api::summary_model_access::access::{ProxyGlobals, check_summary_model_access};
use crate::api::summary_model_access::checks::SummaryModelChecks;
use crate::api::summary_model_access::repositories::SummaryAccessRepositories;
use crate::api::summary_model_limits::budget::{SummaryModelBudgets, summary_model_within_budget};
use crate::api::summary_model_limits::rate::{ProxyHooks, summary_within_rate_limit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDenial {
    Access,
    Budget,
    Rate,
}

impl PolicyDenial {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Access => "summary_model_access_denied",
            Self::Budget => "summary_model_budget_exceeded",
            Self::Rate => "summary_model_rate_limit_exceeded",
        }
    }

    #[must_use]
    pub fn warning(self, summary_model: &str) -> String {
        match self {
            Self::Access => format!(
                "compact_20260112: caller not authorized for summary_model={summary_model}; skipping summary call"
            ),
            Self::Budget => format!(
                "compact_20260112: caller over model budget for summary_model={summary_model}; skipping summary call"
            ),
            Self::Rate => format!(
                "compact_20260112: caller over rate limit for summary_model={summary_model}; skipping summary call"
            ),
        }
    }
}

// access, then budget, then rate; the first denial stops the later checks without stopping the request
pub async fn policy_denial<Checks, Repositories, Budgets, Hooks>(
    principal: Option<&UserAPIKeyAuth>,
    proxy: Option<&ProxyGlobals<'_, Checks, Repositories>>,
    budgets: Option<&Budgets>,
    hooks: Option<&Hooks>,
    summary_model: &str,
) -> Result<Option<PolicyDenial>, SummaryExecutionError<Hooks::Error>>
where
    Checks: SummaryModelChecks,
    Repositories: SummaryAccessRepositories,
    Budgets: SummaryModelBudgets,
    Hooks: ProxyHooks,
{
    let access = check_summary_model_access(principal, summary_model, proxy)
        .await
        .map_err(SummaryExecutionError::Access)?;
    if !access {
        return Ok(Some(PolicyDenial::Access));
    }
    let budget = summary_model_within_budget(principal, budgets, summary_model)
        .await
        .map_err(SummaryExecutionError::Budget)?;
    if !budget {
        return Ok(Some(PolicyDenial::Budget));
    }
    let rate = summary_within_rate_limit(principal, hooks, summary_model)
        .await
        .map_err(SummaryExecutionError::Rate)?;
    Ok((!rate).then_some(PolicyDenial::Rate))
}
