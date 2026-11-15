use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::GuardrailSettings;
use crate::guardrail_initializers::bedrock;
use crate::guardrail_initializers::hide_secrets;
use crate::guardrail_initializers::lakera;
use crate::guardrail_initializers::lakera_v2;
use crate::guardrail_initializers::presidio;
use crate::guardrail_initializers::tool_permission;
use crate::tool_policy::{
    RequestToolNames, ToolPolicyError, ToolPolicyLookup, initialize_tool_policy,
};

use super::callback::{CallbackScoping, CallbackSpec, GuardrailCallback};
use super::error::RegistryError;
use super::integrations;

// Each list is the class's get_supported_event_hooks: a mode outside it is a startup error
const BEDROCK_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::DuringMcpCall,
];

const LAKERA_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
];

const LAKERA_V2_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
];

const PRESIDIO_HOOKS: &[GuardrailEventHooks] = &[
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::LoggingOnly,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::PostMcpCall,
];

const TOOL_PERMISSION_HOOKS: &[GuardrailEventHooks] =
    &[GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];

pub struct GuardrailFactories {
    policies: Arc<dyn ToolPolicyLookup + Send + Sync>,
    names: Arc<dyn RequestToolNames + Send + Sync>,
}

impl GuardrailFactories {
    #[must_use]
    pub fn new(
        policies: Arc<dyn ToolPolicyLookup + Send + Sync>,
        names: Arc<dyn RequestToolNames + Send + Sync>,
    ) -> Self {
        Self { policies, names }
    }

    pub fn build(
        &self,
        kind: &str,
        params: &LitellmParams,
        guardrail_name: &str,
        strict_modes: bool,
    ) -> Result<Vec<GuardrailCallback>, RegistryError> {
        if kind.is_empty() {
            return Err(RegistryError::MissingGuardrailType);
        }
        match kind {
            integrations::BEDROCK => {
                single(bedrock_callback(params, guardrail_name, strict_modes)?)
            }
            integrations::LAKERA => single(lakera_callback(params, guardrail_name, strict_modes)?),
            integrations::LAKERA_V2 => {
                single(lakera_v2_callback(params, guardrail_name, strict_modes)?)
            }
            integrations::PRESIDIO => presidio_callbacks(params, guardrail_name, strict_modes),
            integrations::HIDE_SECRETS => hide_secrets_callbacks(params, guardrail_name),
            integrations::TOOL_PERMISSION => single(tool_permission_callback(
                params,
                guardrail_name,
                strict_modes,
            )?),
            integrations::TOOL_POLICY => {
                self.tool_policy_callback(params, guardrail_name, strict_modes)
            }
            other => Err(RegistryError::Unsupported {
                kind: other.to_owned(),
            }),
        }
    }

    fn tool_policy_callback(
        &self,
        params: &LitellmParams,
        guardrail_name: &str,
        strict_modes: bool,
    ) -> Result<Vec<GuardrailCallback>, RegistryError> {
        let guardrail = initialize_tool_policy(
            params,
            Some(guardrail_name),
            strict_modes,
            Arc::clone(&self.policies),
            Arc::clone(&self.names),
        )
        .map_err(|error| match error {
            ToolPolicyError::Base(base) => RegistryError::Base(base),
        })?;
        let mut callbacks = Vec::with_capacity(1);
        callbacks.push(GuardrailCallback {
            base: guardrail.base.clone(),
            spec: CallbackSpec::ToolPolicy(guardrail),
            scoping: CallbackScoping::default(),
        });
        Ok(callbacks)
    }
}

fn hook_settings(
    name: &str,
    mode: &LitellmParamsMode,
    default_on: GuardrailFlag,
    timeout: Option<f64>,
    hooks: &[GuardrailEventHooks],
) -> GuardrailSettings {
    let mut settings = GuardrailSettings::default();
    settings.guardrail_name = Some(name.to_owned());
    settings.supported_event_hooks = Some(hooks.to_vec());
    settings.event_hook = Some(mode.clone());
    settings.default_on = default_on;
    settings.timeout = timeout;
    settings
}

fn single(callback: GuardrailCallback) -> Result<Vec<GuardrailCallback>, RegistryError> {
    let mut callbacks = Vec::with_capacity(1);
    callbacks.push(callback);
    Ok(callbacks)
}

fn finish(
    settings: GuardrailSettings,
    strict_modes: bool,
    spec: CallbackSpec,
) -> Result<GuardrailCallback, RegistryError> {
    Ok(GuardrailCallback {
        base: GuardrailBase::new(settings, strict_modes)?,
        spec,
        scoping: CallbackScoping::default(),
    })
}

fn bedrock_callback(
    params: &LitellmParams,
    guardrail_name: &str,
    strict_modes: bool,
) -> Result<GuardrailCallback, RegistryError> {
    let args = bedrock::initialize_bedrock(params, Some(guardrail_name))?;
    let mut settings = hook_settings(
        guardrail_name,
        &args.event_hook,
        args.default_on,
        args.timeout,
        BEDROCK_HOOKS,
    );
    settings.mask_request_content = args.mask_request_content;
    settings.mask_response_content = args.mask_response_content;
    settings.only_scan_new_messages = args.only_scan_new_messages;
    finish(settings, strict_modes, CallbackSpec::Bedrock(args))
}

fn lakera_callback(
    params: &LitellmParams,
    guardrail_name: &str,
    strict_modes: bool,
) -> Result<GuardrailCallback, RegistryError> {
    let args = lakera::initialize_lakera(params, Some(guardrail_name));
    let settings = hook_settings(
        guardrail_name,
        &args.event_hook,
        args.default_on,
        args.timeout,
        LAKERA_HOOKS,
    );
    finish(settings, strict_modes, CallbackSpec::Lakera(args))
}

fn lakera_v2_callback(
    params: &LitellmParams,
    guardrail_name: &str,
    strict_modes: bool,
) -> Result<GuardrailCallback, RegistryError> {
    let args = lakera_v2::initialize_lakera_v2(params, Some(guardrail_name))?;
    let settings = hook_settings(
        guardrail_name,
        &args.event_hook,
        args.default_on,
        args.timeout,
        LAKERA_V2_HOOKS,
    );
    finish(settings, strict_modes, CallbackSpec::LakeraV2(args))
}

fn presidio_callbacks(
    params: &LitellmParams,
    guardrail_name: &str,
    strict_modes: bool,
) -> Result<Vec<GuardrailCallback>, RegistryError> {
    let args = presidio::initialize_presidio(params, Some(guardrail_name))?;
    let mut callbacks = Vec::with_capacity(args.len());
    for arg in args {
        let mut settings = hook_settings(
            guardrail_name,
            &arg.event_hook,
            arg.default_on,
            arg.timeout,
            PRESIDIO_HOOKS,
        );
        settings.mask_response_content = arg.mask_response_content;
        callbacks.push(finish(settings, strict_modes, CallbackSpec::Presidio(arg))?);
    }
    Ok(callbacks)
}

fn hide_secrets_callbacks(
    params: &LitellmParams,
    guardrail_name: &str,
) -> Result<Vec<GuardrailCallback>, RegistryError> {
    let Err(error) = hide_secrets::initialize_hide_secrets(params, Some(guardrail_name)) else {
        // the enterprise class is absent until betterlite ports it on its own task
        return Ok(Vec::new());
    };
    Err(RegistryError::Enterprise(error))
}

fn tool_permission_callback(
    params: &LitellmParams,
    guardrail_name: &str,
    strict_modes: bool,
) -> Result<GuardrailCallback, RegistryError> {
    let args = tool_permission::initialize_tool_permission(params, Some(guardrail_name))?;
    let mut settings = hook_settings(
        guardrail_name,
        &args.event_hook,
        args.default_on,
        None,
        TOOL_PERMISSION_HOOKS,
    );
    settings
        .violation_message_template
        .clone_from(&args.violation_message_template);
    finish(settings, strict_modes, CallbackSpec::ToolPermission(args))
}
