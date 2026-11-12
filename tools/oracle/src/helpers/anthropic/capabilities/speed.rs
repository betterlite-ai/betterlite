use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_anthropic::capabilities::speed::{
    SpeedDecision, SpeedParam, SpeedRequest, maybe_drop_speed_param, model_supports_speed_param,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::adapter::Adapter;
use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
pub(super) struct SpeedSupportStep {
    model: String,
    custom_llm_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DropFlags {
    drop_params: bool,
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpeedDropStep {
    model: String,
    custom_llm_provider: Option<String>,
    params: IndexMap<String, Json>,
    #[serde(flatten)]
    drop: DropFlags,
}

impl SpeedSupportStep {
    pub(super) fn answer(&self, adapter: &Adapter<'_>) -> Value {
        match model_supports_speed_param(
            &self.model,
            self.custom_llm_provider.as_deref(),
            &adapter.catalogs,
        ) {
            Ok(supported) => Value::Bool(supported),
            Err(error) => json!({"error": {"status": error.status_code()}}),
        }
    }
}

impl SpeedDropStep {
    pub(super) fn answer(&self, adapter: &Adapter<'_>) -> Result<Value, Box<HelperError>> {
        let speed = self
            .params
            .get("speed")
            .map_or(SpeedParam::Absent, SpeedParam::Present);
        let request = SpeedRequest {
            model: &self.model,
            speed,
            drop: DropParams {
                global: self.drop.global_drop_params,
                call: self.drop.drop_params,
            },
            custom_llm_provider: self.custom_llm_provider.as_deref(),
        };
        let decision = match maybe_drop_speed_param(&request, &adapter.catalogs) {
            Ok(decision) => decision,
            Err(error) => return Ok(json!({"error": {"status": error.status_code()}})),
        };
        let mut params = self.params.clone();
        if decision == SpeedDecision::Drop {
            params.shift_remove("speed");
        }
        let encoded = serde_json::to_value(&params).or_value_error()?;
        Ok(json!({"params": encoded, "warning": adapter.warnings.first()}))
    }
}
