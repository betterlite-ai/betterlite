use std::collections::BTreeMap;

use betterlite_logging::otel::config::{ExporterOwner, ExporterSpec, Settings};
use betterlite_logging::otel::integrations::arize_settings::ArizeConfig;
use betterlite_logging::otel::integrations::levo_settings::LevoConfig;
use betterlite_logging::otel::integrations::phoenix_settings::PhoenixConfig;
use betterlite_logging::otel::integrations::weave_settings::WeaveOtelConfig;
use betterlite_logging::otel::presets::arize::{ArizePreset, arize_dynamic_headers};
use betterlite_logging::otel::presets::base::Preset;
use betterlite_logging::otel::presets::langtrace::LangtracePreset;
use betterlite_logging::otel::presets::levo::LevoPreset;
use betterlite_logging::otel::presets::phoenix::{PhoenixPreset, phoenix_project_headers};
use betterlite_logging::otel::presets::weave::{WeavePreset, weave_dynamic_headers};
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use self::base::BaseInput;

mod base;
mod tests;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ArizeConfig {
        environment: BTreeMap<String, String>,
    },
    ArizePreset {
        environment: BTreeMap<String, String>,
        base: BaseInput,
    },
    ArizeDynamicHeaders {
        params: BTreeMap<String, String>,
    },
    PhoenixConfig {
        environment: BTreeMap<String, String>,
    },
    PhoenixPreset {
        environment: BTreeMap<String, String>,
        base: BaseInput,
    },
    PhoenixProjectHeaders {
        metadata: BTreeMap<String, String>,
    },
    LevoConfig {
        environment: BTreeMap<String, String>,
    },
    LevoPreset {
        environment: BTreeMap<String, String>,
        base: BaseInput,
    },
    WeaveConfig {
        environment: BTreeMap<String, String>,
    },
    WeavePreset {
        environment: BTreeMap<String, String>,
        base: BaseInput,
        #[serde(default)]
        allow_missing_credentials: bool,
    },
    WeaveDynamicHeaders {
        params: BTreeMap<String, String>,
    },
    LangtracePreset {
        base: BaseInput,
    },
}

#[derive(Clone, Copy)]
enum ConfigValue<'source> {
    Arize(&'source ArizeConfig),
    Phoenix(&'source PhoenixConfig),
    Levo(&'source LevoConfig),
    Weave(&'source WeaveOtelConfig),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::ArizeConfig { environment } => Ok(config_json(ConfigValue::Arize(
                &ArizeConfig::from_env(environment),
            ))),
            Input::ArizePreset { environment, base } => {
                preset_value(&ArizePreset, environment, base, false)
            }
            Input::ArizeDynamicHeaders { params } => Ok(json!(arize_dynamic_headers(params))),
            Input::PhoenixConfig { environment } => PhoenixConfig::from_env(environment)
                .map(|config| config_json(ConfigValue::Phoenix(&config)))
                .map_err(|error| invalid(&error.to_string())),
            Input::PhoenixPreset { environment, base } => {
                preset_value(&PhoenixPreset, environment, base, false)
            }
            Input::PhoenixProjectHeaders { metadata } => {
                Ok(json!(phoenix_project_headers(metadata)))
            }
            Input::LevoConfig { environment } => LevoConfig::from_env(environment)
                .map(|config| config_json(ConfigValue::Levo(&config)))
                .map_err(|error| invalid(&error.to_string())),
            Input::LevoPreset { environment, base } => {
                preset_value(&LevoPreset, environment, base, false)
            }
            Input::WeaveConfig { environment } => WeaveOtelConfig::from_env(environment)
                .map(|config| config_json(ConfigValue::Weave(&config)))
                .map_err(|error| invalid(&error.to_string())),
            Input::WeavePreset {
                environment,
                base,
                allow_missing_credentials,
            } => preset_value(&WeavePreset, environment, base, *allow_missing_credentials),
            Input::WeaveDynamicHeaders { params } => Ok(json!(weave_dynamic_headers(params))),
            Input::LangtracePreset { base } => preset_value(
                &LangtracePreset,
                &BTreeMap::<String, String>::new(),
                base,
                false,
            ),
        }
    }
}

fn preset_value(
    preset: &dyn Preset,
    environment: &BTreeMap<String, String>,
    base: &BaseInput,
    allow_missing_credentials: bool,
) -> crate::helpers::HelperResult {
    let settings = preset
        .build(environment, base.settings()?, allow_missing_credentials)
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(settings_value(&settings))
}

fn config_json(config: ConfigValue<'_>) -> Value {
    match config {
        ConfigValue::Arize(values) => json!({
            "space_id": &values.space_id,
            "space_key": &values.space_key,
            "api_key": values.api_key.as_ref().map(ExposeSecret::expose_secret),
            "protocol": values.protocol.exporter_kind(),
            "endpoint": &values.endpoint,
            "project_name": &values.project_name,
        }),
        ConfigValue::Phoenix(values) => json!({
            "otlp_auth_headers": &values.otlp_auth_headers,
            "protocol": values.protocol.exporter_kind(),
            "endpoint": &values.endpoint,
            "project_name": &values.project_name,
        }),
        ConfigValue::Levo(values) => json!({
            "otlp_auth_headers": &values.otlp_auth_headers,
            "protocol": values.protocol.exporter_kind(),
            "endpoint": &values.endpoint,
        }),
        ConfigValue::Weave(values) => json!({
            "otlp_auth_headers": &values.otlp_auth_headers,
            "endpoint": &values.endpoint,
            "project_id": &values.project_id,
            "protocol": values.protocol.exporter_kind(),
        }),
    }
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn settings_value(settings: &Settings) -> Value {
    json!({
        "exporters": settings.exporters.iter().map(exporter_value).collect::<Vec<Value>>(),
        "mapper_names": &settings.mapper_names,
        "resource_attributes": &settings.resource_attributes,
    })
}

fn exporter_value(spec: &ExporterSpec) -> Value {
    json!({
        "kind": &spec.kind,
        "endpoint": &spec.endpoint,
        "traces_endpoint": &spec.traces_endpoint,
        "headers": &spec.headers,
        "owner": spec.owner.map(owner_name),
        "options": &spec.options,
        "use_simple_processor": spec.use_simple_processor,
        "requires_headers": spec.requires_headers,
    })
}

const fn owner_name(owner: ExporterOwner) -> &'static str {
    match owner {
        ExporterOwner::ArizeAx => "arize",
        ExporterOwner::ArizePhoenix => "arize_phoenix",
        ExporterOwner::LangfuseOtel => "langfuse_otel",
        ExporterOwner::WeaveOtel => "weave_otel",
        ExporterOwner::Levo => "levo",
        ExporterOwner::Agentops => "agentops",
        ExporterOwner::Newrelic => "newrelic",
        ExporterOwner::Signoz => "signoz",
    }
}
