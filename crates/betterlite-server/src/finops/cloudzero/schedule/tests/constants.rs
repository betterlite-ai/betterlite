#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger;

use crate::finops::cloudzero::schedule::dry_run::{DEFAULT_DRY_RUN_LIMIT, SAMPLE_LIMIT};
use crate::finops::cloudzero::schedule::{
    DEFAULT_INTERVAL_MINUTES, DEFAULT_MAX_FETCHED_RECORDS, DEFAULT_TIMEZONE, EXPORT_JOB_NAME,
    ExportOperation,
};

use super::fixtures::{FakeTransport, datastore, logger, settings};

#[test]
fn cloudzero_schedule() {
    assert_eq!(EXPORT_JOB_NAME, "cloudzero_export_usage_data");
    assert_eq!(DEFAULT_TIMEZONE, "UTC");
    assert_eq!(DEFAULT_INTERVAL_MINUTES, 60);
    assert_eq!(DEFAULT_MAX_FETCHED_RECORDS, 50_000);
    assert_eq!(DEFAULT_DRY_RUN_LIMIT, 10_000);
    assert_eq!(SAMPLE_LIMIT, 50);
    assert_eq!(ExportOperation::default(), ExportOperation::ReplaceHourly);
    assert_eq!(ExportOperation::default().as_str(), "replace_hourly");
    assert_eq!(ExportOperation::Sum.as_str(), "sum");
    let assembled = logger(settings(), datastore(), FakeTransport::default(), None);
    let contract: Arc<dyn CallbackLogger> = Arc::new(assembled);
    assert_eq!(contract.callback_name(), "cloudzero");
}
