use betterlite_core::api::claude_gateway_policy::{
    ClaudeGatewayPolicyError, ClaudeGatewayPolicyInput, GatewayGeneralSettings,
    ensure_gateway_enabled, evaluate,
};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    State,
    Guard,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    mode: Mode,
    general_settings: GatewayGeneralSettings,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let policy = ClaudeGatewayPolicyInput {
            general_settings: input.general_settings.clone(),
        };
        match input.mode {
            Mode::State => {
                let result = evaluate(&policy);
                Ok(serde_json::json!({
                    "enabled": result.enabled,
                    "managed_settings": result.managed_settings,
                }))
            }
            Mode::Guard => {
                let result = ensure_gateway_enabled(&policy).map_err(failure)?;
                Ok(serde_json::json!({"managed_settings": result.managed_settings}))
            }
        }
    }
}

fn failure(error: ClaudeGatewayPolicyError) -> Box<crate::helpers::HelperError> {
    let ClaudeGatewayPolicyError::Disabled {
        status_code,
        detail,
    } = error;
    Box::new(crate::helpers::HelperError {
        class: "HTTPException".to_owned(),
        status: Some(status_code),
        error_type: None,
        param: None,
        code: None,
        message: detail.to_owned(),
    })
}
