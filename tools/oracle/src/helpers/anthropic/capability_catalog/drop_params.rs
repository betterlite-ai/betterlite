use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_anthropic::anthropic_capability_catalog::forced_tool::apply_forced_tool_choice;
use betterlite_anthropic::anthropic_capability_catalog::sampling::{
    SamplingParam, SamplingRequest, apply_sampling_param,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::adapter::Adapter;
use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
pub(super) struct ForcedToolStep {
    model: String,
    tool_choice: Json,
    drop_params: bool,
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct SamplingStep {
    model: String,
    param: SamplingParam,
    value: Json,
    output_key: String,
    drop_params: bool,
    global_drop_params: bool,
}

impl ForcedToolStep {
    pub(super) fn answer(&self, adapter: &Adapter<'_>) -> Value {
        let drop = DropParams {
            global: self.global_drop_params,
            call: self.drop_params,
        };
        match apply_forced_tool_choice(&self.model, &self.tool_choice, drop, &adapter.catalogs) {
            Ok(choice) => json!({"tool_choice": choice, "warning": adapter.warnings.first()}),
            Err(error) => json!({"error": {"status": error.status_code()}}),
        }
    }
}

impl SamplingStep {
    pub(super) fn answer(&self, adapter: &Adapter<'_>) -> Result<Value, Box<HelperError>> {
        let request = SamplingRequest {
            model: &self.model,
            param: self.param,
            value: &self.value,
            drop: DropParams {
                global: self.global_drop_params,
                call: self.drop_params,
            },
        };
        match apply_sampling_param(&request, &adapter.catalogs) {
            Ok(forwarded) => {
                let mut params = IndexMap::new();
                if let Some(value) = forwarded {
                    params.insert(self.output_key.clone(), value);
                }
                serde_json::to_value(&params).or_value_error()
            }
            Err(error) => Ok(json!({"error": {"status": error.status_code()}})),
        }
    }
}
