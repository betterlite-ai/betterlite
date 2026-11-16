use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::repello_content::SUPPORTED_EVENT_HOOKS;

use super::RepelloAILifecycle;
use super::config::{RepelloAIParams, RepelloAISettings, event_hook_from_mode};
use super::error::RepelloLifecycleError;
use super::stream::{SilentConsole, SilentLogger};

// LiteLLM's Repello hooks are undecorated, so _call_analyze's finally is the only record it writes.
const REPELLO_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: true,
    uses_apply_guardrail_interface: false,
    deny_text_on_error: false,
};

pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<RepelloAILifecycle>);
}

// LiteLLM's initialize_guardrail: the callback joins the logging callback manager before it is returned.
pub fn initialize_guardrail<E, R>(
    params: &RepelloAIParams,
    env: &E,
    client_builder: reqwest::ClientBuilder,
    registry: &mut R,
) -> Result<Arc<RepelloAILifecycle>, RepelloLifecycleError>
where
    E: EnvironmentLookup,
    R: CallbackRegistry,
{
    let event_hook = event_hook_from_mode(&params.mode)?;
    let settings = RepelloAISettings::resolve(params, env)?;
    let client = client_builder
        .build()
        .map_err(|source| RepelloLifecycleError::Client { source })?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| RepelloLifecycleError::Redactor { source })?;
    let hooks = VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    };
    let base = GuardrailBase::new(guardrail_settings(params, event_hook), params.strict_modes)?;
    let guardrail = Arc::new(RepelloAILifecycle {
        base,
        settings,
        client,
        clock: SystemTime::now,
        hooks,
    });
    registry.add_litellm_callback(Arc::clone(&guardrail));
    Ok(guardrail)
}

#[must_use]
pub fn guardrail_settings(
    params: &RepelloAIParams,
    event_hook: LitellmParamsMode,
) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: params.guardrail_name.clone(),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: Some(event_hook),
        default_on: params.default_on,
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
        timeout: params.timeout,
        switches: REPELLO_SWITCHES,
    }
}
