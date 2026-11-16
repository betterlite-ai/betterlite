use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

use super::error::NomaLifecycleError;
use super::{LEGACY_NOMA_DEPRECATION_WARNED, NomaGuardrail, SUPPORTED_EVENT_HOOKS};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::noma_scan_policy::{DEFAULT_API_BASE, NomaScanPolicy, NomaSettings};
use crate::noma_v2::config::{
    API_BASE_ENV, API_KEY_ENV, APPLICATION_ID_ENV, BLOCK_FAILURES_ENV, MONITOR_MODE_ENV,
};

pub const ANONYMIZE_INPUT_ENV: &str = "NOMA_ANONYMIZE_INPUT";

// What LiteLLM's constructor receives: the configured values, before the environment fills the gaps
#[derive(Debug, Clone, Default)]
pub struct NomaParams {
    pub guardrail_name: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub application_id: Option<String>,
    pub monitor_mode: Option<bool>,
    pub block_failures: Option<bool>,
    pub anonymize_input: Option<bool>,
    pub timeout: Option<f64>,
    pub event_hook: Option<LitellmParamsMode>,
    pub default_on: bool,
}

// The outside world a guardrail is built from: the HTTP client, the clock, the environment and the task owner
pub struct NomaDependencies<'environment> {
    pub client: reqwest::Client,
    pub now: fn() -> SystemTime,
    pub env: &'environment dyn EnvironmentLookup,
    pub tasks: TaskTracker,
    pub strict_modes: bool,
}

fn resolved_flag(
    configured: Option<bool>,
    env: &dyn EnvironmentLookup,
    name: &str,
    default: bool,
) -> Result<bool, NomaLifecycleError> {
    if let Some(flag) = configured {
        return Ok(flag);
    }
    let Some(text) = env.lookup(name) else {
        return Ok(default);
    };
    Ok(betterlite_env::flag(name, &text)?.unwrap_or(default))
}

fn resolved_settings(
    params: &NomaParams,
    env: &dyn EnvironmentLookup,
) -> Result<NomaSettings, NomaLifecycleError> {
    let timeout = params
        .timeout
        .map(|seconds| {
            Duration::try_from_secs_f64(seconds)
                .map_err(|source| NomaLifecycleError::Timeout { seconds, source })
        })
        .transpose()?;
    // an environment variable set to the empty string counts as unset
    let api_key = params.api_key.clone().or_else(|| {
        env.lookup(API_KEY_ENV)
            .filter(|key| !key.is_empty())
            .map(SecretString::from)
    });
    let api_base = params
        .api_base
        .clone()
        .or_else(|| env.lookup(API_BASE_ENV).filter(|base| !base.is_empty()))
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let application_id = params
        .application_id
        .clone()
        .or_else(|| env.lookup(APPLICATION_ID_ENV).filter(|id| !id.is_empty()));
    Ok(NomaSettings {
        guardrail_name: params.guardrail_name.clone(),
        event_hook: params.event_hook.clone(),
        api_key,
        api_base: Some(api_base),
        application_id,
        monitor_mode: resolved_flag(params.monitor_mode, env, MONITOR_MODE_ENV, false)?,
        anonymize_input: resolved_flag(params.anonymize_input, env, ANONYMIZE_INPUT_ENV, false)?,
        timeout,
    })
}

fn base_settings(params: &NomaParams) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: params.guardrail_name.clone(),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: params.event_hook.clone(),
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
        switches: GuardrailSwitches::default(),
    }
}

impl NomaGuardrail {
    pub fn new(
        params: &NomaParams,
        dependencies: NomaDependencies<'_>,
    ) -> Result<Self, NomaLifecycleError> {
        if !LEGACY_NOMA_DEPRECATION_WARNED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "Guardrail provider 'noma' is deprecated. Please migrate to 'noma_v2'. The legacy 'noma' API will no longer be supported after March 31, 2026."
            );
        }
        let settings = resolved_settings(params, dependencies.env)?;
        let base = GuardrailBase::new(base_settings(params), dependencies.strict_modes)
            .map_err(|source| NomaLifecycleError::Base { source })?;
        Ok(Self {
            base,
            block_failures: resolved_flag(
                params.block_failures,
                dependencies.env,
                BLOCK_FAILURES_ENV,
                true,
            )?,
            policy: NomaScanPolicy {
                settings,
                client: dependencies.client,
                now: dependencies.now,
            },
            tasks: dependencies.tasks,
        })
    }
}
