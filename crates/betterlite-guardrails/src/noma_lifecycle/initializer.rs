use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::enums::SupportedGuardrailIntegrations;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use tokio_util::task::TaskTracker;

use super::config::{NomaDependencies, NomaParams};
use super::error::NomaLifecycleError;
use super::{NomaCallback, NomaGuardrail};
use crate::noma_v2::config::NomaV2Params;
use crate::noma_v2::guardrail::NomaV2Guardrail;
use crate::noma_v2::{
    CallbackRegistry as NomaV2CallbackRegistry, DEFAULT_CONNECT_TIMEOUT,
    initialize_guardrail_v2 as initialize_noma_v2,
};
use crate::third_party_init_a::decode;
use crate::third_party_init_a::error::ThirdPartyInitAError;

// LiteLLM's litellm.logging_callback_manager
pub trait NomaRegistry {
    fn add_litellm_callback(&mut self, callback: NomaCallback);
}

// What building a guardrail needs from outside: the environment, the HTTP client's builder and the task owner
pub struct BuildContext<'context> {
    pub env: &'context dyn EnvironmentLookup,
    pub client_builder: reqwest::ClientBuilder,
    pub tasks: TaskTracker,
    pub strict_modes: bool,
}

pub struct InitContext<'context> {
    pub build: BuildContext<'context>,
    pub registry: &'context mut dyn NomaRegistry,
}

// What a registry entry receives: the configured parameters and the name the guardrail was listed under
#[derive(Clone, Copy)]
pub struct GuardrailRequest<'request> {
    pub params: &'request LitellmParams,
    pub guardrail_name: &'request str,
}

// The callback a registry entry builds, registered or not
pub type Built = Result<NomaCallback, NomaLifecycleError>;

pub type Initializer = fn(GuardrailRequest<'_>, InitContext<'_>) -> Built;

pub type GuardrailClass = fn(GuardrailRequest<'_>, BuildContext<'_>) -> Built;

pub const GUARDRAIL_INITIALIZER_REGISTRY: [(SupportedGuardrailIntegrations, Initializer); 2] = [
    (SupportedGuardrailIntegrations::Noma, initialize_guardrail),
    (
        SupportedGuardrailIntegrations::NomaV2,
        initialize_guardrail_v2,
    ),
];

pub const GUARDRAIL_CLASS_REGISTRY: [(SupportedGuardrailIntegrations, GuardrailClass); 2] = [
    (SupportedGuardrailIntegrations::Noma, build_legacy_guardrail),
    (SupportedGuardrailIntegrations::NomaV2, build_v2_guardrail),
];

// noma_v2 reads its environment through a generic bound, so the trait object needs a sized stand-in
struct DynEnvironment<'environment>(&'environment dyn EnvironmentLookup);

impl EnvironmentLookup for DynEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0.lookup(name)
    }
}

// noma_v2 registers the guardrail it builds, so building it alone hands it a registry that keeps nothing
struct DiscardRegistry;

impl NomaV2CallbackRegistry for DiscardRegistry {
    fn add_litellm_callback(&mut self, _guardrail: Arc<NomaV2Guardrail>) {}
}

impl NomaParams {
    pub fn from_litellm(
        params: &LitellmParams,
        guardrail_name: &str,
    ) -> Result<Self, NomaLifecycleError> {
        let fields = &params.base.fields;
        let extra = &params.base.extra.0;
        Ok(Self {
            guardrail_name: Some(guardrail_name.to_owned()),
            api_key: fields.api_key.clone(),
            api_base: fields.api_base.clone(),
            application_id: decode("application_id", extra.get("application_id"))?,
            monitor_mode: decode("monitor_mode", extra.get("monitor_mode"))?,
            block_failures: decode("block_failures", extra.get("block_failures"))?,
            anonymize_input: decode("anonymize_input", extra.get("anonymize_input"))?,
            timeout: fields.timeout,
            event_hook: Some(params.mode.clone()),
            default_on: fields.default_on.unwrap_or_default(),
        })
    }
}

// use_v2 is a bool, or text read as a bool the way every environment string is
fn use_v2(params: &LitellmParams) -> Result<bool, NomaLifecycleError> {
    match params.base.extra.0.get("use_v2") {
        None | Some(GuardrailValue::Null) => Ok(false),
        Some(GuardrailValue::Boolean(flag)) => Ok(*flag),
        Some(GuardrailValue::Text(text)) => {
            Ok(betterlite_env::flag("use_v2", text)?.unwrap_or(false))
        }
        Some(_wrong) => Err(NomaLifecycleError::Params(
            ThirdPartyInitAError::InvalidParam {
                name: "use_v2".to_owned(),
            },
        )),
    }
}

// The NomaGuardrail class: building one without registering it
pub fn build_legacy_guardrail(request: GuardrailRequest<'_>, context: BuildContext<'_>) -> Built {
    let configured = NomaParams::from_litellm(request.params, request.guardrail_name)?;
    Ok(NomaCallback::Legacy(Arc::new(NomaGuardrail::new(
        &configured,
        dependencies(context)?,
    )?)))
}

fn dependencies(context: BuildContext<'_>) -> Result<NomaDependencies<'_>, NomaLifecycleError> {
    let client = context
        .client_builder
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .build()
        .map_err(|source| NomaLifecycleError::Client { source })?;
    Ok(NomaDependencies {
        client,
        now: SystemTime::now,
        env: context.env,
        tasks: context.tasks,
        strict_modes: context.strict_modes,
    })
}

// The NomaV2Guardrail class: B3782 builds it and registers it, so the registry it gets here is a throwaway
pub fn build_v2_guardrail(request: GuardrailRequest<'_>, context: BuildContext<'_>) -> Built {
    let configured =
        NomaV2Params::from_litellm(request.params, request.guardrail_name, context.strict_modes)?;
    Ok(NomaCallback::V2(initialize_noma_v2(
        &configured,
        &DynEnvironment(context.env),
        context.client_builder,
        &mut DiscardRegistry,
    )?))
}

// Adding the callback to litellm.logging_callback_manager, then handing it back to the caller
pub fn initialize_guardrail(request: GuardrailRequest<'_>, context: InitContext<'_>) -> Built {
    if use_v2(request.params)? {
        return initialize_guardrail_v2(request, context);
    }
    let callback = build_legacy_guardrail(request, context.build)?;
    context.registry.add_litellm_callback(callback.clone());
    Ok(callback)
}

pub fn initialize_guardrail_v2(request: GuardrailRequest<'_>, context: InitContext<'_>) -> Built {
    let callback = build_v2_guardrail(request, context.build)?;
    context.registry.add_litellm_callback(callback.clone());
    Ok(callback)
}
