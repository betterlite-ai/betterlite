use betterlite_server::shadow_eval_job_loading::jobs::ActiveShadowEvalJob;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::inputs::{JobInput, ValidateCall};

pub(crate) fn validate(call: &ValidateCall) -> Value {
    let jobs: Vec<Value> = call
        .rows
        .iter()
        .map(|input| validate_row(input, &call.arm_router))
        .collect();
    json!({"jobs": jobs})
}

fn validate_row(input: &JobInput, arm_router: &str) -> Value {
    let Ok(job) = ActiveShadowEvalJob::from_row(&input.to_row(), 0, Decimal::ZERO) else {
        return json!({
            "id": input.id,
            "valid": false,
            "direction": Value::Null,
            "arm_router_names": Vec::<String>::new(),
            "arm_target": Value::Null,
        });
    };
    json!({
        "id": job.id,
        "valid": true,
        "direction": job.direction.as_str(),
        "arm_router_names": job.arm_router_names,
        "arm_target": job.arm_target(arm_router),
    })
}
