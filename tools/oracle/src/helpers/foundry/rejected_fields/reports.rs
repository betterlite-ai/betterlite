use betterlite_foundry::rejected_fields::{FoundryRepair, repair};
use serde_json::{Value, json};

use super::input::{RepairCall, TriggerCall};

pub(super) fn trigger_report(call: &TriggerCall) -> Value {
    let policy = FoundryRepair {
        call: call.request_drop_params,
        global: call.global_drop_params,
    };
    json!({"retry": policy.should_retry(&call.error)})
}

pub(super) fn repair_report(call: &RepairCall) -> Value {
    match repair(&call.error, &call.body) {
        Ok(body) => json!({"body": body}),
        Err(error) => json!({"error": error.to_string()}),
    }
}
