pub mod error;

mod bridge;
mod scan;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::vigil_runtime::Settings as EnvironmentDefaults;
use secrecy::ExposeSecret as _;

use crate::generic_guardrail_contracts::{GuardrailInputType, UnreachableFallback};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::third_party_init_b::vigil_guard::VigilGuardArgs;
use crate::vigil_guard::DEFAULT_TIMEOUT;
use crate::vigil_guard::config::{VigilGuardGuardrailConfigModel, VigilGuardSettings};
use crate::vigil_guard::error::VigilGuardError;
use crate::vigil_guard::handler_protocol::{AsyncPostHandler, RequestTimeout};
use error::VigilRuntimeError;

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 2] =
    [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];
pub const DEFAULT_CONNECT_SECONDS: f64 = 5.0;

// The class defines apply_guardrail and sets none of CustomGuardrail's native-hook class variables
const VIGIL_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: false,
    uses_apply_guardrail_interface: true,
    deny_text_on_error: false,
};

pub struct VigilGuardParams {
    pub args: VigilGuardArgs,
    pub defaults: EnvironmentDefaults,
    pub supported_event_hooks: Option<Vec<GuardrailEventHooks>>,
    pub strict_modes: bool,
    pub handler: Arc<dyn AsyncPostHandler + Send + Sync>,
}

pub struct VigilGuard {
    pub base: GuardrailBase,
    pub settings: VigilGuardSettings,
    pub unreachable_fallback: UnreachableFallback,
    pub handler: Arc<dyn AsyncPostHandler + Send + Sync>,
}

impl VigilGuard {
    pub fn new(params: VigilGuardParams) -> Result<Self, VigilRuntimeError> {
        let settings = resolved_settings(&params.args, &params.defaults)?;
        let base = base_for(&params)?;
        Ok(Self {
            base,
            settings,
            unreachable_fallback: fallback_mode(&params.args.unreachable_fallback),
            handler: params.handler,
        })
    }
}

fn base_for(params: &VigilGuardParams) -> Result<GuardrailBase, VigilRuntimeError> {
    let supported_event_hooks = params
        .supported_event_hooks
        .clone()
        .unwrap_or_else(|| SUPPORTED_EVENT_HOOKS.to_vec());
    GuardrailBase::new(
        GuardrailSettings {
            guardrail_name: (!params.args.name.is_empty()).then(|| params.args.name.clone()),
            supported_event_hooks: Some(supported_event_hooks),
            event_hook: Some(params.args.event_hook.clone()),
            default_on: params.args.default_on,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: VIGIL_SWITCHES,
        },
        params.strict_modes,
    )
    .map_err(VigilRuntimeError::from)
}

fn resolved_settings(
    args: &VigilGuardArgs,
    defaults: &EnvironmentDefaults,
) -> Result<VigilGuardSettings, VigilRuntimeError> {
    let api_base = args
        .api_base
        .as_deref()
        .filter(|base| !base.is_empty())
        .or(defaults.api_base.as_deref())
        .ok_or(VigilGuardError::MissingConfig { field: "api_base" })?;
    let api_key = args
        .api_key
        .clone()
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| defaults.api_key.clone())
        .ok_or(VigilGuardError::MissingConfig { field: "api_key" })?;
    Ok(VigilGuardSettings {
        api_base: api_base.trim_end_matches('/').to_owned(),
        api_key,
        timeout: resolved_timeout(args.timeout)?,
    })
}

fn resolved_timeout(seconds: Option<f64>) -> Result<RequestTimeout, VigilRuntimeError> {
    let Some(seconds) = seconds else {
        return Ok(DEFAULT_TIMEOUT);
    };
    let total = Duration::try_from_secs_f64(seconds)
        .map_err(|source| VigilRuntimeError::Timeout { seconds, source })?;
    let connect = Duration::try_from_secs_f64(seconds.min(DEFAULT_CONNECT_SECONDS))
        .map_err(|source| VigilRuntimeError::Timeout { seconds, source })?;
    Ok(RequestTimeout { total, connect })
}

fn fallback_mode(text: &str) -> UnreachableFallback {
    if text.eq_ignore_ascii_case("fail_open") {
        return UnreachableFallback::FailOpen;
    }
    UnreachableFallback::FailClosed
}

impl GuardrailContract for VigilGuard {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        scan::apply(
            self,
            inputs,
            request_data,
            input_type,
            logging_obj.as_deref(),
        )
        .await
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        Some(GuardrailValue::Object(BTreeMap::from([
            (
                "model".to_owned(),
                GuardrailValue::Text("VigilGuardGuardrailConfigModel".to_owned()),
            ),
            (
                "ui_friendly_name".to_owned(),
                GuardrailValue::Text(VigilGuardGuardrailConfigModel::ui_friendly_name().to_owned()),
            ),
        ])))
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}
