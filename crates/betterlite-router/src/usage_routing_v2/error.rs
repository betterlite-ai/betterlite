use std::fmt;

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
#[derive(Clone, Debug, Eq, PartialEq)]
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

// A counter the pinned dict prints as None, and a limit it prints as float("inf") when the deployment sets none.
struct Counted(Option<i64>);

impl fmt::Display for Counted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(value) => value.fmt(formatter),
            None => formatter.write_str("None"),
        }
    }
}

struct Limited(Option<Decimal>);

impl fmt::Display for Limited {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(value) => value.fmt(formatter),
            None => formatter.write_str("inf"),
        }
    }
}

impl fmt::Display for NoDeploymentAvailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "No deployments available for selected model. Passed model={}. Deployments=",
            self.model
        )?;
        formatter.write_str("{")?;
        for (position, load) in self.deployments.iter().enumerate() {
            if position > 0 {
                formatter.write_str(", ")?;
            }
            write!(
                formatter,
                "'{}': {{'current_tpm': {}, 'tpm_limit': {}, 'current_rpm': {}, 'rpm_limit': {}}}",
                load.id,
                Counted(load.current_tpm),
                Limited(load.tpm_limit),
                Counted(load.current_rpm),
                Limited(load.rpm_limit),
            )?;
        }
        formatter.write_str("}")
    }
}

impl std::error::Error for NoDeploymentAvailable {}

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

// _return_potential_deployments adds the usage to the limit, so LiteLLM fails the whole selection with a TypeError
// when a limit is not a number; betterlite names the deployment and the field instead.
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
