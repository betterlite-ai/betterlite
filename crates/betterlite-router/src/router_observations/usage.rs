use chrono::{DateTime, Utc};

use super::RouterObservations;
use super::counter::{CounterError, Increment};
use super::keys::{ROUTER_COUNTER_TTL, usage_keys};

// the deployment's configured limits, folded from the deployment, its litellm_params and its model_info by the catalog lookup
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeploymentUsage {
    pub tpm: Option<i64>,
    pub rpm: Option<i64>,
    pub params_tpm: Option<i64>,
    pub params_rpm: Option<i64>,
    pub model_info_tpm: Option<i64>,
    pub model_info_rpm: Option<i64>,
    // deployment_has_io_token_limits: an IO deployment still records tpm/rpm load in mixed model groups
    pub io_token_limits: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageIncrement<'increment> {
    pub deployment_id: &'increment str,
    pub deployment_name: &'increment str,
    pub total_tokens: i64,
    pub rpm_increment: i64,
}

// nothing to track only when neither tpm/rpm nor itpm/otpm limits are set
fn tracked(deployment: &DeploymentUsage) -> bool {
    let limits = [
        deployment.tpm,
        deployment.rpm,
        deployment.params_tpm,
        deployment.params_rpm,
        deployment.model_info_tpm,
        deployment.model_info_rpm,
    ];
    !limits.iter().all(Option::is_none) || deployment.io_token_limits
}

impl RouterObservations<'_> {
    #[must_use]
    pub fn increment_deployment_usage(
        &self,
        increment: &UsageIncrement<'_>,
        deployment: Option<&DeploymentUsage>,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, CounterError> {
        let Some(deployment) = deployment else {
            return Ok(None);
        };
        if !tracked(deployment) {
            return Ok(None);
        }
        if increment.total_tokens <= 0 && increment.rpm_increment <= 0 {
            return Ok(None);
        }
        let usage = usage_keys(increment.deployment_id, increment.deployment_name, now);
        let operations = [
            Increment {
                key: usage.tpm.clone(),
                value: increment.total_tokens,
                ttl: ROUTER_COUNTER_TTL,
            },
            Increment {
                key: usage.rpm,
                value: increment.rpm_increment,
                ttl: ROUTER_COUNTER_TTL,
            },
        ];
        self.counters.increment_pipeline(&operations)?;
        Ok(Some(usage.tpm))
    }
}
