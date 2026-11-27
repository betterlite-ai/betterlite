use std::convert::Infallible;
use std::sync::{Mutex, PoisonError};

use betterlite_core::api::batch_input_accounting::calls::BatchAccountingCallError;
use betterlite_core::api::batch_input_accounting::error::BatchInputAccountingError;
use betterlite_core::api::batch_input_accounting::result::BatchInputAccountingResult;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::json;

use super::scripted::{ScriptedCounter, ScriptedFiles};

pub(super) type AccountingError = BatchInputAccountingError<
    BatchAccountingCallError<Infallible, Infallible>,
    Infallible,
    Infallible,
>;

pub(super) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BatchInputAccountingError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn accounting_failure(error: &AccountingError) -> Box<HelperError> {
    let mut helper_error = failure(error.to_string());
    helper_error.status = error.http().map(|http| http.status);
    helper_error
}

fn snapshot<T>(values: &Mutex<Vec<T>>) -> Vec<T>
where
    T: Clone,
{
    values
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(super) fn record_report(
    outcome: Result<BatchInputAccountingResult, AccountingError>,
    files: &ScriptedFiles,
    counter: &ScriptedCounter,
) -> HelperResult {
    let result = outcome.map_err(|error| accounting_failure(&error))?;
    let usage = serde_json::to_value(&result).map_err(|error| failure(error.to_string()))?;
    Ok(json!({
        "usage": usage,
        "fetched": snapshot(&files.fetched),
        "counted": snapshot(&counter.counted),
    }))
}
