use std::collections::BTreeMap;

use betterlite_models::capture_rate::alert::alert_message;
use betterlite_models::capture_rate::compute::{CaptureRateRequest, compute_capture_rate};
use betterlite_models::capture_rate::report::{CaptureRateReport, CheckResult};
use betterlite_models::capture_rate::settings::SpendCaptureProvider;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = serde_json::Map::new();
        for (name, case) in &input.cases {
            answers.insert(name.clone(), answer(case)?);
        }
        Ok(serde_json::Value::Object(answers))
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    cases: BTreeMap<String, Case>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Case {
    Report(ReportCase),
    AlertReport(ReportCase),
    AlertCredentialMissing {
        provider: SpendCaptureProvider,
        env_var: String,
    },
    AlertRequestFailed {
        provider: SpendCaptureProvider,
        detail: String,
    },
}

#[derive(Debug, Deserialize)]
struct ReportCase {
    provider: SpendCaptureProvider,
    start_date: NaiveDate,
    end_date: NaiveDate,
    threshold: Decimal,
    captured_by_day: BTreeMap<String, Decimal>,
    billed_by_day: BTreeMap<String, Decimal>,
}

fn report(case: &ReportCase) -> Result<CaptureRateReport, Box<HelperError>> {
    compute_capture_rate(&CaptureRateRequest {
        provider: case.provider,
        start_date: case.start_date,
        end_date: case.end_date,
        captured_by_day: &case.captured_by_day,
        billed_by_day: &case.billed_by_day,
        threshold: case.threshold,
    })
    .or_value_error()
}

fn answer(case: &Case) -> Result<serde_json::Value, Box<HelperError>> {
    match case {
        Case::Report(report_case) => Ok(serde_json::json!({"report": report(report_case)?})),
        Case::AlertReport(report_case) => Ok(serde_json::json!({
            "alert": alert_message(&CheckResult::Report(report(report_case)?)),
        })),
        Case::AlertCredentialMissing { provider, env_var } => {
            let missing = CheckResult::CredentialMissing {
                provider: *provider,
                env_var: env_var.clone(),
            };
            Ok(serde_json::json!({"alert": alert_message(&missing)}))
        }
        Case::AlertRequestFailed { provider, detail } => {
            let failed = CheckResult::RequestFailed {
                provider: *provider,
                detail: detail.clone(),
            };
            Ok(serde_json::json!({"alert": alert_message(&failed)}))
        }
    }
}
