use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::deployment_identity::spec::DeploymentSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptionalPreCallCheck {
    DeploymentAffinity,
    ResponsesApiDeploymentCheck,
    SessionAffinity,
    EncryptedContentAffinity,
    PromptCaching,
    RouterBudgetLimiting,
    EnforceModelRateLimits,
}

// _RUNTIME_TOGGLEABLE_PRE_CALL_CHECKS: only these are removed when a later setting leaves them out.
pub const RUNTIME_TOGGLEABLE: [OptionalPreCallCheck; 3] = [
    OptionalPreCallCheck::PromptCaching,
    OptionalPreCallCheck::EnforceModelRateLimits,
    OptionalPreCallCheck::EncryptedContentAffinity,
];

impl OptionalPreCallCheck {
    // The OptionalPreCallChecks literal of litellm/types/router.py; a name outside it registers nothing.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "deployment_affinity" => Some(Self::DeploymentAffinity),
            "responses_api_deployment_check" => Some(Self::ResponsesApiDeploymentCheck),
            "session_affinity" => Some(Self::SessionAffinity),
            "encrypted_content_affinity" => Some(Self::EncryptedContentAffinity),
            "prompt_caching" => Some(Self::PromptCaching),
            "router_budget_limiting" => Some(Self::RouterBudgetLimiting),
            "enforce_model_rate_limits" => Some(Self::EnforceModelRateLimits),
            _ => None,
        }
    }
}

// The checks add_optional_pre_call_checks has registered; the callbacks themselves belong to their own services.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnabledChecks {
    pub checks: BTreeSet<OptionalPreCallCheck>,
}

impl EnabledChecks {
    // add_optional_pre_call_checks unions the requested names and registers each callback at most once.
    pub fn add(&mut self, requested: &[OptionalPreCallCheck]) {
        for check in requested {
            self.checks.insert(*check);
        }
    }

    // set_optional_pre_call_checks drops the toggleable checks the new setting leaves out, then adds what it names.
    pub fn replace(&mut self, requested: Option<&[OptionalPreCallCheck]>) {
        let Some(requested) = requested else {
            return;
        };
        for check in RUNTIME_TOGGLEABLE {
            if !requested.contains(&check) {
                self.checks.remove(&check);
            }
        }
        self.add(requested);
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum PreCallError {
    #[error("deployment {deployment_id} is over its limit")]
    RateLimited { deployment_id: String, status: u16 },
    #[error("{message}")]
    Failed { message: String },
}

#[async_trait]
pub trait DeploymentPreCallCheck: Send + Sync {
    fn check(&self, deployment: &DeploymentSpec) -> Result<(), PreCallError>;

    async fn async_check(&self, deployment: &DeploymentSpec) -> Result<(), PreCallError>;
}

// The deployment's cooldown after a rate-limited pre-call check; the cooldown service itself is B307.
pub trait CooldownRecorder: Send + Sync {
    fn record_rate_limit(&self, deployment_id: &str, status: u16);
}

// routing_strategy_checks: every registered check runs in order and the first failure stops the request.
pub fn routing_strategy_checks(
    deployment: &DeploymentSpec,
    checks: &[&dyn DeploymentPreCallCheck],
) -> Result<(), PreCallError> {
    let mut remaining = checks.iter();
    while let Some(check) = remaining.next() {
        check.check(deployment)?;
    }
    Ok(())
}

fn rate_limit(failure: &PreCallError) -> Option<(&str, u16)> {
    match failure {
        PreCallError::RateLimited {
            deployment_id,
            status,
        } => Some((deployment_id.as_str(), *status)),
        PreCallError::Failed { .. } => None,
    }
}

// async_routing_strategy_checks: a rate limit records the cooldown before the same error goes up.
pub async fn async_routing_strategy_checks(
    deployment: &DeploymentSpec,
    checks: &[&dyn DeploymentPreCallCheck],
    cooldown: &dyn CooldownRecorder,
) -> Result<(), PreCallError> {
    for check in checks {
        if let Err(failure) = check.async_check(deployment).await {
            if let Some((deployment_id, status)) = rate_limit(&failure) {
                cooldown.record_rate_limit(deployment_id, status);
            }
            return Err(failure);
        }
    }
    Ok(())
}
