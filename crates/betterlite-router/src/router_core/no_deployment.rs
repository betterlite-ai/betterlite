use serde_json::Number;
use thiserror::Error;

// RouterErrorTypes, which the proxy reads back off a RouterRateLimitError.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoDeploymentType {
    RateLimit,
    AllInCooldown,
}

impl NoDeploymentType {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::RateLimit => "rate_limit_error",
            Self::AllInCooldown => "all_deployments_in_cooldown",
        }
    }
}

// The typed RouterRateLimitError async_raise_no_deployment_exception returns when no deployment is left.
#[derive(Clone, Debug, Error, PartialEq)]
#[error("{message}")]
pub struct NoDeployment {
    message: String,
    pub error_type: NoDeploymentType,
    pub model: String,
    pub cooldown_time: f64,
    pub enable_pre_call_checks: bool,
    pub cooldown_list: Vec<String>,
    pub model_ids: Vec<String>,
}

impl NoDeployment {
    #[must_use]
    pub fn new(
        model: String,
        cooldown_time: f64,
        enable_pre_call_checks: bool,
        cooldown_list: Vec<String>,
        model_ids: Vec<String>,
    ) -> Self {
        let all_in_cooldown = !model_ids.is_empty()
            && model_ids
                .iter()
                .all(|id| cooldown_list.iter().any(|cooling| cooling == id));
        let error_type = if all_in_cooldown {
            NoDeploymentType::AllInCooldown
        } else {
            NoDeploymentType::RateLimit
        };
        let mut message = format!(
            "No deployments available for selected model, Try again in {} seconds.",
            number_text(cooldown_time)
        );
        if all_in_cooldown {
            message.push_str(" All deployments for selected model are in cooldown.");
        }
        message.push_str(&format!(
            " Passed model={model}. pre-call-checks={enable_pre_call_checks}, cooldown_list={cooldown_list:?}"
        ));
        Self {
            message,
            error_type,
            model,
            cooldown_time,
            enable_pre_call_checks,
            cooldown_list,
            model_ids,
        }
    }
}

// LiteLLM interpolates the float with str(), which is the shortest text that reads back as the same number
fn number_text(value: f64) -> String {
    Number::from_f64(value)
        .unwrap_or_else(|| Number::from(0))
        .to_string()
}
