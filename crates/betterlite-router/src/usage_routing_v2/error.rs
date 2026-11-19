use betterlite_error::rate_limit::RateLimitCategory;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::routing_value_store::StoreError;

// The counters and the limits async_get_available_deployments reports for one deployment when nothing is left.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentLoad {
    pub id: String,
    pub current_tpm: Option<i64>,
    pub tpm_limit: Option<Decimal>,
    pub current_rpm: Option<i64>,
    pub rpm_limit: Option<Decimal>,
}

// The litellm.RateLimitError async_get_available_deployments raises once every deployment is past a limit.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error(
    "No deployments available for selected model. Passed model={}. Deployments={}",
    .model,
    deployments_text(.deployments)
)]
pub struct NoDeploymentAvailable {
    pub model: String,
    pub deployments: Vec<DeploymentLoad>,
}

impl NoDeploymentAvailable {
    pub const STATUS_CODE: u16 = 429;
    pub const CODE: &'static str = "429";
    pub const ERROR_TYPE: &'static str = "throttling_error";
    pub const RETRY_AFTER_SECONDS: u64 = 60;
    pub const CATEGORY: RateLimitCategory = RateLimitCategory::VendorRateLimit;
}

// The Python dict repr of the raised error: a None counter prints None and a missing limit prints inf.
fn deployments_text(loads: &[DeploymentLoad]) -> String {
    let mut text = String::from("{");
    for (position, load) in loads.iter().enumerate() {
        if position > 0 {
            text.push_str(", ");
        }
        text.push('\'');
        text.push_str(&load.id);
        text.push_str("': {'current_tpm': ");
        match load.current_tpm {
            Some(count) => text.push_str(&count.to_string()),
            None => text.push_str("None"),
        }
        text.push_str(", 'tpm_limit': ");
        match load.tpm_limit {
            Some(limit) => text.push_str(&limit.to_string()),
            None => text.push_str("inf"),
        }
        text.push_str(", 'current_rpm': ");
        match load.current_rpm {
            Some(count) => text.push_str(&count.to_string()),
            None => text.push_str("None"),
        }
        text.push_str(", 'rpm_limit': ");
        match load.rpm_limit {
            Some(limit) => text.push_str(&limit.to_string()),
            None => text.push_str("inf"),
        }
        text.push('}');
    }
    text.push('}');
    text
}

// usage_counter_keys builds its key from the deployment id, which the store rejects only past its key limit.
#[derive(Debug, Error)]
pub enum KeyError {
    #[error("the routing key of deployment {deployment} is past the store's key limit")]
    TooLong {
        deployment: String,
        #[source]
        source: StoreError,
    },
}

// _return_potential_deployments adds the usage to the limit, so a limit that is not a number names its field here.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum LimitError {
    #[error("deployment {deployment} has a {field} limit that is not a number")]
    NonNumeric {
        deployment: String,
        field: &'static str,
    },
}

impl LimitError {
    // the field the pipeline's metric error names
    #[must_use]
    pub const fn field(&self) -> &'static str {
        match self {
            Self::NonNumeric { field, .. } => field,
        }
    }
}

// Every way reading the counters or ranking the pool stops a selection.
#[derive(Debug, Error)]
pub enum SelectionError {
    #[error(transparent)]
    NoDeployment(#[from] NoDeploymentAvailable),
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error(transparent)]
    Limit(#[from] LimitError),
}
