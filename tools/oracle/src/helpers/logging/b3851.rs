use std::collections::BTreeMap;

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_logging::otel::config::Settings;
use betterlite_logging::otel::presets::agentops::AgentopsPreset;
use betterlite_logging::otel::presets::base::Preset;
use betterlite_logging::otel::presets::destinations::{
    destination_capable_backends, destination_for,
};
use betterlite_logging::otel::presets::langfuse::{
    LangfusePreset, langfuse_dynamic_headers,
};
use betterlite_logging::otel::presets::newrelic::{
    NewrelicPreset, newrelic_dynamic_endpoint, newrelic_dynamic_headers,
};
use betterlite_logging::otel::presets::signoz::{
    SignozPreset, signoz_dynamic_endpoint, signoz_dynamic_headers,
};
use betterlite_logging::otel::presets::{
    dynamic_otlp_endpoint, dynamic_otlp_headers, preset_for, project_routing_headers,
};
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use self::base::BaseInput;
use self::values::{destination_value, headers_value, settings_value};

mod base;
mod tests;
mod values;

pub(crate) struct Op;

#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Function {
    AgentopsPreset,
    DestinationCapableBackends,
    DestinationFor,
    DynamicOtlpEndpoint,
    DynamicOtlpHeaders,
    LangfuseDynamicHeaders,
    LangfusePreset,
    NewrelicDynamicEndpoint,
    NewrelicDynamicHeaders,
    NewrelicPreset,
    PresetFor,
    ProjectRoutingHeaders,
    SignozDynamicEndpoint,
    SignozDynamicHeaders,
    SignozPreset,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    base: Option<BaseInput>,
    #[serde(default)]
    allow_missing_credentials: bool,
    #[serde(default)]
    params: Option<Json>,
    #[serde(default)]
    allowed_hosts: Vec<String>,
    #[serde(default)]
    callback_name: Option<String>,
    #[serde(default)]
    auth_metadata: Option<BTreeMap<String, String>>,
    #[serde(default)]
    service_name: Option<String>,
}

impl Input {
    fn json_params(&self) -> &Json {
        self.params.as_ref().unwrap_or(&Json::Null)
    }

    fn string_params(&self) -> BTreeMap<String, String> {
        let Json::Object(fields) = self.json_params() else {
            return BTreeMap::new();
        };
        let mut values = BTreeMap::new();
        for (name, value) in fields {
            if let Json::Text(text) = value {
                values.insert(name.clone(), text.clone());
            }
        }
        values
    }

    fn callback_name(&self) -> &str {
        self.callback_name.as_deref().unwrap_or_default()
    }

    fn hosts(&self) -> Vec<AllowedDestination> {
        self.allowed_hosts
            .iter()
            .filter_map(|entry| AllowedDestination::parse(entry))
            .collect()
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let values = input.string_params();
        let hosts = input.hosts();
        let callback_name = Some(input.callback_name());
        match input.function {
            Function::AgentopsPreset => preset_result(&AgentopsPreset, input, false),
            Function::NewrelicPreset => preset_result(&NewrelicPreset, input, false),
            Function::SignozPreset => preset_result(&SignozPreset, input, false),
            Function::LangfusePreset => {
                preset_result(&LangfusePreset, input, input.allow_missing_credentials)
            }
            Function::NewrelicDynamicHeaders
            | Function::NewrelicDynamicEndpoint
            | Function::SignozDynamicHeaders
            | Function::SignozDynamicEndpoint
            | Function::LangfuseDynamicHeaders => {
                builder_result(input.function, &values, &hosts)
            }
            Function::PresetFor => Ok(json!(preset_for(input.callback_name()).is_some())),
            Function::DynamicOtlpHeaders => Ok(dynamic_otlp_headers(
                callback_name,
                input.json_params(),
                &hosts,
            )
            .map_or(Value::Null, |headers| headers_value(&headers))),
            Function::DynamicOtlpEndpoint => {
                let endpoint = dynamic_otlp_endpoint(callback_name, input.json_params(), &hosts);
                Ok(json!(endpoint))
            }
            Function::ProjectRoutingHeaders => Ok(headers_value(&project_routing_headers(
                callback_name,
                input.auth_metadata.as_ref(),
            ))),
            Function::DestinationCapableBackends => {
                let backends: Vec<&str> = destination_capable_backends().into_iter().collect();
                Ok(json!(backends))
            }
            Function::DestinationFor => Ok(destination_result(input, &hosts)),
        }
    }
}

fn builder_result(
    function: Function,
    values: &BTreeMap<String, String>,
    hosts: &[AllowedDestination],
) -> crate::helpers::HelperResult {
    match function {
        Function::NewrelicDynamicHeaders => {
            Ok(headers_value(&newrelic_dynamic_headers(values)))
        }
        Function::NewrelicDynamicEndpoint => Ok(json!(newrelic_dynamic_endpoint(values))),
        Function::SignozDynamicHeaders => {
            Ok(headers_value(&signoz_dynamic_headers(values, hosts)))
        }
        Function::SignozDynamicEndpoint => Ok(json!(signoz_dynamic_endpoint(values, hosts))),
        Function::LangfuseDynamicHeaders => {
            Ok(headers_value(&langfuse_dynamic_headers(values)))
        }
        Function::AgentopsPreset
        | Function::DestinationCapableBackends
        | Function::DestinationFor
        | Function::DynamicOtlpEndpoint
        | Function::DynamicOtlpHeaders
        | Function::LangfusePreset
        | Function::NewrelicPreset
        | Function::PresetFor
        | Function::ProjectRoutingHeaders
        | Function::SignozPreset => Err(invalid("not a builder function")),
    }
}

fn preset_result(
    preset: &dyn Preset,
    input: &Input,
    allow_missing_credentials: bool,
) -> crate::helpers::HelperResult {
    let Some(base) = input.base.as_ref() else {
        return Err(invalid("the preset needs a base config"));
    };
    let settings: Settings = preset
        .build(
            &input.environment,
            base.settings(),
            allow_missing_credentials,
        )
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(settings_value(&settings))
}

fn destination_result(input: &Input, hosts: &[AllowedDestination]) -> Value {
    let destination = destination_for(
        &input.environment,
        hosts,
        input.callback_name(),
        input.json_params(),
        input.service_name.as_deref(),
    );
    destination
        .as_ref()
        .map_or(Value::Null, destination_value)
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
