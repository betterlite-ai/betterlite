use betterlite_error::proxy_rate_limit_error::map_v3_rate_limit_type;
use betterlite_error::rate_limit::{ProxyRateLimitError, ProxyRateLimitErrorInit};
use betterlite_rate_limit::provider_attribution::{
    ModelGroupLimits, PriorityValue, convert_priority_to_percent,
};
use betterlite_redact::structured::Payload;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    MapType {
        value: Option<String>,
    },
    CoerceMessage {
        detail: Payload,
    },
    PriorityPercent {
        value: PriorityValue,
        model_info: Option<ModelGroupLimits>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MapType { value } => Ok(json!({
                "rate_limit_type": map_v3_rate_limit_type(value.as_deref()),
            })),
            Input::CoerceMessage { detail } => {
                let error = ProxyRateLimitError::new(ProxyRateLimitErrorInit {
                    detail: detail.clone(),
                    headers: None,
                    category: None,
                    rate_limit_type: None,
                    model: None,
                    llm_provider: None,
                })
                .or_config_error()?;
                Ok(json!({ "message": error.message }))
            }
            Input::PriorityPercent { value, model_info } => Ok(json!({
                "percent": convert_priority_to_percent(*value, model_info.as_ref()),
            })),
        }
    }
}
