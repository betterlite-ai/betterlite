use betterlite_core::cost::context::ProviderInference;
use betterlite_models::resolution::ProviderIdentity;
use serde_json::{Value, json};

use super::{Call, Failure, Function};

mod breakdown;
mod identity;
mod response_time;

pub(super) fn run_call(call: &Call) -> Value {
    match execute(call) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "error": {"class": error.class}}),
    }
}

fn execute(call: &Call) -> Result<Value, Failure> {
    match call.function {
        Function::Identity => identity::identity(call),
        Function::ResponseTime => response_time::response_time(call),
        Function::Breakdown => breakdown::breakdown(call),
    }
}

struct NoInference;

impl ProviderInference for NoInference {
    type Failure = &'static str;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, Self::Failure> {
        Err("the case supplies the provider")
    }
}
