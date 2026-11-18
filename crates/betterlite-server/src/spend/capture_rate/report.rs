use std::error::Error as StdError;

use betterlite_models::capture_rate::OPENAI_BILLED_PROVIDERS;
use betterlite_models::capture_rate::compute::{CaptureRateRequest, compute_capture_rate};
use betterlite_models::capture_rate::report::CheckResult;
use betterlite_models::capture_rate::settings::SpendCaptureProvider;
use betterlite_provider::billing::{
    BillingError, OPENAI_ADMIN_KEY_ENV_VAR, fetch_openai_daily_costs,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::store::CapturedSpendRequest;
use super::{CaptureRateJob, CaptureRateJobError};

// One provider's window and threshold; project IDs scope the provider bill only.
pub struct ProviderCheck<'ids> {
    pub provider: SpendCaptureProvider,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub threshold: Decimal,
    pub openai_project_ids: &'ids [String],
}

// The credential is read before billing, and a failed bill skips the captured-spend query entirely.
pub async fn capture_rate_report<E>(
    job: &CaptureRateJob<'_, E>,
    check: &ProviderCheck<'_>,
) -> Result<CheckResult, CaptureRateJobError<E>>
where
    E: StdError + Send + Sync + 'static,
{
    let provider = check.provider;
    let Some(admin_key) = job
        .credentials
        .admin_key()
        .map_err(CaptureRateJobError::Credentials)?
    else {
        return Ok(missing_credential(provider));
    };
    let billed = match fetch_openai_daily_costs(
        check.start_date,
        check.end_date,
        &admin_key,
        check.openai_project_ids,
        job.http_get,
    )
    .await
    {
        Ok(billed) => billed,
        Err(error) => return Ok(request_failed(provider, &error)),
    };
    let captured = job
        .spend
        .captured_by_day(&CapturedSpendRequest {
            providers: &OPENAI_BILLED_PROVIDERS,
            start_date: check.start_date,
            end_date: check.end_date,
        })
        .await?;
    let report = compute_capture_rate(&CaptureRateRequest {
        provider: check.provider,
        start_date: check.start_date,
        end_date: check.end_date,
        captured_by_day: &captured,
        billed_by_day: &billed,
        threshold: check.threshold,
    })?;
    Ok(CheckResult::Report(report))
}

fn missing_credential(provider: SpendCaptureProvider) -> CheckResult {
    CheckResult::CredentialMissing {
        provider,
        env_var: OPENAI_ADMIN_KEY_ENV_VAR.to_owned(),
    }
}

fn request_failed(provider: SpendCaptureProvider, error: &BillingError) -> CheckResult {
    CheckResult::RequestFailed {
        provider,
        detail: error.to_string(),
    }
}
