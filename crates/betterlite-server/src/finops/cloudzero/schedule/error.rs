use std::error::Error;

use betterlite_db::finops::cloudzero::CloudZeroUsageError;
use betterlite_env::EnvError;
use betterlite_logging::finops::cloudzero::delivery::error::DeliveryError;
use thiserror::Error;

use super::dry_run::DryRunError;
use super::usage_rows::UsageRowError;

#[derive(Debug, Error)]
pub enum CloudZeroSettingsError {
    #[error(transparent)]
    Environment(#[from] EnvError),
}

#[derive(Debug, Error)]
pub enum CloudZeroExportError<Failure>
where
    Failure: Error + 'static,
{
    #[error(
        "CloudZero configuration missing. Please set CLOUDZERO_API_KEY and CLOUDZERO_CONNECTION_ID environment variables."
    )]
    MissingConfiguration,
    #[error("the usage data could not be read")]
    Usage(#[from] CloudZeroUsageError),
    #[error("the usage rows could not be converted")]
    Rows(#[from] UsageRowError),
    #[error("the CloudZero billing drop could not be sent")]
    Delivery(#[from] DeliveryError<Failure>),
    #[error("the export window is outside the supported date range")]
    WindowOutOfRange,
    #[error("the dry run preview could not be built")]
    Preview(#[from] DryRunError),
}
