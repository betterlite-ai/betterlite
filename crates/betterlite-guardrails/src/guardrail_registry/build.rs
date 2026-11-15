use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_initializers::bedrock::{self, BedrockArgs};
use crate::guardrail_initializers::hide_secrets;
use crate::guardrail_initializers::lakera::{self, LakeraArgs};
use crate::guardrail_initializers::lakera_v2::{self, LakeraV2Args};
use crate::guardrail_initializers::presidio::{self, PresidioArgs};
use crate::guardrail_initializers::tool_permission::{self, ToolPermissionArgs};
use crate::tool_policy::error::ToolPolicyError;
use crate::tool_policy::{RequestToolNames, ToolPolicyLookup, initialize_tool_policy};

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

// What each args struct adds to the base settings, as LiteLLM's constructors do with setdefault
trait HookArgs {
    const HOOKS: &'static [GuardrailEventHooks];

    fn common(&self) -> (&LitellmParamsMode, GuardrailFlag, Option<f64>);

    fn extras(&self, _settings: &mut GuardrailSettings) {}
}

impl HookArgs for BedrockArgs {
    const HOOKS: &'static [GuardrailEventHooks] = BEDROCK_HOOKS;

    fn common(&self) -> (&LitellmParamsMode, GuardrailFlag, Option<f64>) {
        (&self.event_hook, self.default_on, self.timeout)
    }

    fn extras(&self, settings: &mut GuardrailSettings) {
        settings.mask_request_content = self.mask_request_content;
        settings.mask_response_content = self.mask_response_content;
        settings.only_scan_new_messages = self.only_scan_new_messages;
    }
}

impl HookArgs for LakeraArgs {
    const HOOKS: &'static [GuardrailEventHooks] = LAKERA_HOOKS;

    fn common(&self) -> (&LitellmParamsMode, GuardrailFlag, Option<f64>) {
        (&self.event_hook, self.default_on, self.timeout)
    }
}

impl HookArgs for LakeraV2Args {
    const HOOKS: &'static [GuardrailEventHooks] = LAKERA_V2_HOOKS;

    fn common(&self) -> (&LitellmParamsMode, GuardrailFlag, Option<f64>) {
        (&self.event_hook, self.default_on, self.timeout)
    }
}

impl HookArgs for PresidioArgs {
    const HOOKS: &'static [GuardrailEventHooks] = PRESIDIO_HOOKS;

    fn common(&self) -> (&LitellmParamsMode, GuardrailFlag, Option<f64>) {
        (&self.event_hook, self.default_on, self.timeout)
    }

    fn extras(&self, settings: &mut GuardrailSettings) {
        settings.mask_response_content = self.mask_response_content;
    }
}

// LiteLLM's tool permission constructor passes no timeout
impl HookArgs for ToolPermissionArgs {
    const HOOKS: &'static [GuardrailEventHooks] = TOOL_PERMISSION_HOOKS;

    fn common(&self) -> (&LitellmParamsMode, GuardrailFlag, Option<f64>) {
        (&self.event_hook, self.default_on, None)
    }

    fn extras(&self, settings: &mut GuardrailSettings) {
        settings
            .violation_message_template
            .clone_from(&self.violation_message_template);
    }
}

impl GuardrailFactories {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub const fn new(
        policies: Arc<dyn ToolPolicyLookup + Send + Sync>,
        names: Arc<dyn RequestToolNames + Send + Sync>,
    ) -> Self {
        Self { policies, names }
    }

    pub fn build(
        &self,
        kind: &str,
        params: &LitellmParams,
        name: &str,
        strict_modes: bool,
    ) -> Result<Vec<GuardrailCallback>, RegistryError> {
        if kind.is_empty() {
            return Err(RegistryError::MissingGuardrailType);
        }
        match kind {
            integrations::BEDROCK => {
                let args = bedrock::initialize_bedrock(params, Some(name))?;
                Ok(vec![callback(args, name, strict_modes)?])
            }
            integrations::LAKERA => {
                let args = lakera::initialize_lakera(params, Some(name));
                Ok(vec![callback(args, name, strict_modes)?])
            }
            integrations::LAKERA_V2 => {
                let args = lakera_v2::initialize_lakera_v2(params, Some(name))?;
                Ok(vec![callback(args, name, strict_modes)?])
            }
            integrations::PRESIDIO => presidio::initialize_presidio(params, Some(name))?
                .into_iter()
                .map(|args| callback(args, name, strict_modes))
                .collect(),
            integrations::TOOL_PERMISSION => {
                let args = tool_permission::initialize_tool_permission(params, Some(name))?;
                Ok(vec![callback(args, name, strict_modes)?])
            }
            // the enterprise class is absent until betterlite ports it on its own task
            integrations::HIDE_SECRETS => hide_secrets::initialize_hide_secrets(params, Some(name))
                .map(|_args| Vec::new())
                .map_err(RegistryError::Enterprise),
            integrations::TOOL_POLICY => self.tool_policy_callbacks(params, name, strict_modes),
            other => Err(RegistryError::Unsupported {
                kind: other.to_owned(),
            }),
        }
    }

    fn tool_policy_callbacks(
        &self,
        params: &LitellmParams,
        name: &str,
        strict_modes: bool,
    ) -> Result<Vec<GuardrailCallback>, RegistryError> {
        let guardrail = initialize_tool_policy(
            params,
            Some(name),
            strict_modes,
            Arc::clone(&self.policies),
            Arc::clone(&self.names),
        )
        .map_err(|ToolPolicyError::Base(base)| RegistryError::Base(base))?;
        Ok(vec![GuardrailCallback {
            base: guardrail.base.clone(),
            spec: CallbackSpec::ToolPolicy(guardrail),
            scoping: CallbackScoping::default(),
        }])
    }
}

fn callback<A>(args: A, name: &str, strict_modes: bool) -> Result<GuardrailCallback, RegistryError>
where
    A: HookArgs,
    CallbackSpec: From<A>,
{
    let (event_hook, default_on, timeout) = args.common();
    let mut settings = GuardrailSettings {
        guardrail_name: Some(name.to_owned()),
        supported_event_hooks: Some(A::HOOKS.to_vec()),
        event_hook: Some(event_hook.clone()),
        default_on,
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
        timeout,
        switches: GuardrailSwitches::default(),
    };
    args.extras(&mut settings);
    Ok(GuardrailCallback {
        base: GuardrailBase::new(settings, strict_modes)?,
        spec: CallbackSpec::from(args),
        scoping: CallbackScoping::default(),
    })
}
