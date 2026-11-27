use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::callback_settings::names::CallbackName;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use super::otel::shared::SharedOtel;
use super::otel::{legacy, maybe};
use super::{ArmBuild, CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::otel::callbacks::agentops::transport::AuthTransport;
use crate::otel::callbacks::agentops::{AgentOpsCallback, AgentOpsConfig};
use crate::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtel, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
    init_legacy_otel,
};

pub const CALLBACK_NAME: &str = "agentops";
const RESOURCE_ATTRIBUTES_ENV: &str = "OTEL_RESOURCE_ATTRIBUTES";

// The startup step: AgentOps fetches its JWT before the logger exists, and only a listed name builds one (agentops.py 56-65)
pub async fn connect_agentops(
    inputs: &CatalogInputs,
    transport: &dyn AuthTransport,
) -> AgentOpsCallback {
    let config = AgentOpsConfig::from_env(inputs.env.as_ref());
    // With the switch on, V2 answers or the arm refuses its name, so its legacy logger never runs.
    let legacy_runs = matches!(maybe::v2_enabled(inputs), Ok(false));
    if !configured(inputs) || !legacy_runs {
        return AgentOpsCallback { config, auth: None };
    }
    AgentOpsCallback::connect(config, transport).await
}

// set_callbacks only builds the callbacks the config lists, in any of the three lists
fn configured(inputs: &CatalogInputs) -> bool {
    let settings = &inputs.config.sections.callback_settings;
    settings
        .callbacks
        .iter()
        .chain(settings.success_callback.iter())
        .chain(settings.failure_callback.iter())
        .any(|listed| listed == CALLBACK_NAME)
}

// litellm_logging's agentops arm: V2 first, then the AgentOps already in memory, then the connected one
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
    connection: Option<&AgentOpsCallback>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if let Some(logger) = maybe::v2(inputs, runtime, CALLBACK_NAME, in_memory)? {
        return Ok(logger);
    }
    if let Some(existing) = in_memory
        .iter()
        .find(|logger| logger.callback_name() == CALLBACK_NAME)
    {
        return Ok(Arc::clone(existing));
    }
    // a key or team callback naming agentops never reached the startup step, and the resolver cannot fetch
    let Some(startup) = connection.filter(|_| configured(inputs)) else {
        return Err(CatalogError::Unavailable {
            name: CALLBACK_NAME.to_owned(),
        });
    };
    let otel = legacy_otel(inputs, shared, startup)?;
    legacy::complete(inputs, shared, otel)
}

// with_agentops hands the connected callback to the arm through the catalog's own build closure
#[must_use]
pub(super) fn wrap(dispatch: ArmBuild, callback: AgentOpsCallback) -> ArmBuild {
    Arc::new(
        move |name: CallbackName,
              inputs: &CatalogInputs,
              runtime: Option<&LoggerRuntime>,
              shared: &SharedOtel,
              in_memory: &mut Vec<Arc<dyn CallbackLogger>>|
              -> Result<Arc<dyn CallbackLogger>, CatalogError> {
            if name == CallbackName::Agentops {
                let logger = build(inputs, runtime, shared, in_memory, Some(&callback))?;
                maybe::push_once(in_memory, &logger);
                return Ok(logger);
            }
            dispatch(name, inputs, runtime, shared, in_memory)
        },
    )
}

// B8746's legacy path with the AgentOps exporter config and its resource attributes in the environment
pub(crate) fn legacy_otel(
    inputs: &CatalogInputs,
    shared: &SharedOtel,
    connection: &AgentOpsCallback,
) -> Result<LegacyOtel, CatalogError> {
    let environment = resource_environment(shared, connection);
    let init = LegacyOtelInit {
        config: Some(connection.otel_config(&shared.env)),
        callback_name: Some(CALLBACK_NAME.to_owned()),
        overrides: MetadataKeyOverrides::default(),
        injected: InjectedProviders::default(),
        max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
    };
    let otel_environment = LegacyOtelEnvironment {
        env: &environment,
        clients: shared.clients(inputs)?,
        shared_exporter: shared.shared_exporter.clone(),
        globals: &shared.globals,
        proxy: Some(&shared.proxy),
    };
    init_legacy_otel(init, &otel_environment).map_err(|source| CatalogError::Init {
        message: source.to_string(),
    })
}

// the resource gets AgentOps' identity through OTEL_RESOURCE_ATTRIBUTES, with the operator's entries appended and winning
fn resource_environment(
    shared: &SharedOtel,
    connection: &AgentOpsCallback,
) -> BTreeMap<String, String> {
    let mut environment = shared.env.clone();
    let mut attributes: Vec<String> = connection
        .resource_attributes()
        .iter()
        .map(|(name, value)| format!("{name}={}", utf8_percent_encode(value, NON_ALPHANUMERIC)))
        .collect();
    if let Some(operator) = environment
        .get(RESOURCE_ATTRIBUTES_ENV)
        .filter(|value| !value.is_empty())
    {
        attributes.push(operator.clone());
    }
    environment.insert(RESOURCE_ATTRIBUTES_ENV.to_owned(), attributes.join(","));
    environment
}

mod tests;
