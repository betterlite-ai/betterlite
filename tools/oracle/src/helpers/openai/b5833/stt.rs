use serde_json::{Value, json};

use betterlite_openai::groq_stt::{GroqSttError, MapParams, map_openai_params_stt};

use super::Call;

pub(super) fn stt_one(call: &Call) -> Result<Value, GroqSttError> {
    let params = MapParams {
        non_default: &call.non_default,
        optional_params: &call.optional_params,
        drop_params: call.drop_params.unwrap_or(false),
        global_drop_params: call.global_drop_params.unwrap_or(false),
    };
    let optional_params = map_openai_params_stt(&params)?;
    Ok(json!({"optional_params": optional_params}))
}
