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

#[derive(Clone, Copy, Debug, Deserialize)]
struct DropFlags {
    drop_params: bool,
    global_drop_params: bool,
}

impl From<DropFlags> for DropParams {
    fn from(flags: DropFlags) -> Self {
        Self {
            global: flags.global_drop_params,
            call: flags.drop_params,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ForcedToolStep {
    model: String,
    tool_choice: Json,
    #[serde(flatten)]
    drop: DropFlags,
}

#[derive(Debug, Deserialize)]
pub(super) struct SamplingStep {
    model: String,
    param: SamplingParam,
    value: Json,
    output_key: String,
    #[serde(flatten)]
    drop: DropFlags,
}

impl ForcedToolStep {
    pub(super) fn answer(&self, adapter: &Adapter<'_>) -> Value {
        let drop = DropParams::from(self.drop);
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
            drop: DropParams::from(self.drop),
        };
        let forwarded = match apply_sampling_param(&request, &adapter.catalogs) {
            Ok(forwarded) => forwarded,
            Err(error) => return Ok(json!({"error": {"status": error.status_code()}})),
        };
        let mut params = IndexMap::new();
        if let Some(value) = forwarded {
            params.insert(self.output_key.clone(), value);
        }
        let encoded = serde_json::to_value(&params).or_value_error()?;
        Ok(json!({"params": encoded, "warning": adapter.warnings.first()}))
    }
}
