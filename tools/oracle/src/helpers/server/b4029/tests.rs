#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

use super::Op;

const OPERATION: &str = "b4029";

#[rstest]
fn replays_the_user_spend_alert_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/user-spend-alerts/*.yaml")]
    path: PathBuf,
) {
    assert_eq!(replay_case(&path), Ok(()));
}

#[test]
fn rejects_an_unknown_input_field() {
    let input = serde_json::json!({
        "row": {"user_id": "user-1", "daily_spend": 1.0_f64, "monthly_spend": 1.0_f64, "baseline_spend": 0.0_f64},
        "settings": {
            "daily_threshold": null,
            "monthly_threshold": null,
            "spend_anomaly_multiplier": 3.0_f64,
            "spend_anomaly_baseline_days": 7_i32,
            "spend_anomaly_min_spend": 10.0_f64,
        },
        "today": "2026-01-15",
        "thresholds_enabled": true,
        "anomalies_enabled": true,
        "unknown": true,
    });
    let error = Op::decode_and_run(&input).expect_err("the input names an unknown field");
    assert_eq!(error.class, "InvalidInput");
}

fn replay_case(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let case: HelperCase = serde_saphyr::from_str(&source).map_err(|error| error.to_string())?;
    validate_helper(path, &case).map_err(|error| error.to_string())?;
    if case.call.operation != OPERATION {
        return Err(format!(
            "the case names {} instead of {OPERATION}",
            case.call.operation
        ));
    }
    match run_case(&[crate::GROUP], &case, false).map_err(|error| error.to_string())? {
        HelperRun::Match => Ok(()),
        HelperRun::Difference => Err(format!("{} does not match LiteLLM", path.display())),
    }
}
