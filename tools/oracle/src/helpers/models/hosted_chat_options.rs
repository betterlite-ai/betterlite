use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::Sections;
use betterlite_config::sections::hosted_chat_settings::Settings as HostedSettings;
use betterlite_models::baseten_endpoint;
use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::hosted_chat_options::aiml;
use betterlite_openai::hosted_chat_options::aliases::CompatibleChatAlias;
use betterlite_openai::hosted_chat_options::baseten::{self, BasetenSettings};
use betterlite_openai::hosted_chat_options::cerebras::{self, CerebrasSettings};
use betterlite_openai::hosted_chat_options::featherless::{self, DropParams, FeatherlessSettings};
use betterlite_openai::hosted_chat_options::map_renaming_token_limit;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Supported {
        host: ChatHost,
        #[serde(default)]
        model: String,
        #[serde(default)]
        supports_reasoning: bool,
    },
    Map {
        host: ChatHost,
        #[serde(default)]
        model: String,
        config: BTreeMap<String, Value>,
        params: Vec<(String, Value)>,
        #[serde(default)]
        existing: Vec<(String, Value)>,
        #[serde(default)]
        drop_params: bool,
        #[serde(default)]
        supports_reasoning: bool,
    },
    Info {
        host: InfoHost,
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
    },
    Validate {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        headers: Option<BTreeMap<String, String>>,
    },
    BasetenModels {
        models: Vec<String>,
    },
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum ChatHost {
    Baseten,
    Cerebras,
    Featherless,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum InfoHost {
    Featherless,
    Aiml,
    Empower,
    Friendliai,
    Galadriel,
    Github,
}

struct MapRequest<'call> {
    host: ChatHost,
    config: &'call BTreeMap<String, Value>,
    params: &'call [(String, Value)],
    existing: &'call [(String, Value)],
    drop_params: bool,
    supports_reasoning: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut scenarios = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            scenarios.push(scenario_value(scenario)?);
        }
        Ok(json!(scenarios))
    }
}

fn scenario_value(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let sections = ConfigLoader {
        lookup: scenario.environment.clone(),
    }
    .load("")
    .map_err(|error| helper_error(error.to_string()))?
    .sections;
    let steps = scenario
        .calls
        .iter()
        .map(|call| step_value(call, &sections))
        .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
    Ok(json!({"steps": steps}))
}

fn step_value(call: &Call, sections: &Sections) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Supported {
            host,
            supports_reasoning,
            ..
        } => Ok(supported_value(*host, *supports_reasoning)),
        Call::Map {
            host,
            config,
            params,
            existing,
            drop_params,
            supports_reasoning,
            ..
        } => {
            let request = MapRequest {
                host: *host,
                config,
                params,
                existing,
                drop_params: *drop_params,
                supports_reasoning: *supports_reasoning,
            };
            map_value(&request, sections)
        }
        Call::Info {
            host,
            api_base,
            api_key,
        } => Ok(info_value(
            *host,
            api_base.as_deref(),
            api_key.as_deref(),
            sections,
        )),
        Call::Validate { api_key, headers } => {
            Ok(validate_value(api_key.as_deref(), headers.clone()))
        }
        Call::BasetenModels { models } => Ok(baseten_models_value(models)),
    }
}

fn supported_value(host: ChatHost, supports_reasoning: bool) -> Value {
    match host {
        ChatHost::Baseten => projected(json!(baseten::SUPPORTED_PARAMS)),
        ChatHost::Cerebras => projected(json!(cerebras::supported_params(supports_reasoning))),
        ChatHost::Featherless => projected(json!(featherless::SUPPORTED_PARAMS)),
    }
}

fn map_value(request: &MapRequest<'_>, sections: &Sections) -> Result<Value, Box<HelperError>> {
    match request.host {
        ChatHost::Baseten => token_limit_map::<BasetenSettings>(request, baseten::SUPPORTED_PARAMS),
        ChatHost::Cerebras => token_limit_map::<CerebrasSettings>(
            request,
            &cerebras::supported_params(request.supports_reasoning),
        ),
        ChatHost::Featherless => featherless_map(request, &sections.hosted_chat_settings),
    }
}

fn token_limit_map<Settings>(
    request: &MapRequest<'_>,
    supported: &[&str],
) -> Result<Value, Box<HelperError>>
where
    Settings: serde::Serialize + serde::de::DeserializeOwned,
{
    let settings: Settings = deserialize_settings(request.config)?;
    let snapshot = snapshot(&settings)?;
    let mut options: IndexMap<String, Value> = request.existing.iter().cloned().collect();
    map_renaming_token_limit(
        &request.params.iter().cloned().collect(),
        &mut options,
        supported,
    );
    Ok(mapped_value(snapshot, &options))
}

fn featherless_map(
    request: &MapRequest<'_>,
    host_settings: &HostedSettings,
) -> Result<Value, Box<HelperError>> {
    let settings: FeatherlessSettings = deserialize_settings(request.config)?;
    let snapshot = snapshot(&settings)?;
    let mut options: IndexMap<String, Value> = request.existing.iter().cloned().collect();
    let drop = DropParams {
        argument: request.drop_params,
        global: host_settings.drop_params,
    };
    match featherless::map_openai_params(
        &request.params.iter().cloned().collect(),
        &mut options,
        drop,
    ) {
        Ok(()) => Ok(mapped_value(snapshot, &options)),
        Err(error) => Ok(rejected(error.status_code())),
    }
}

fn info_value(
    host: InfoHost,
    api_base: Option<&str>,
    api_key: Option<&str>,
    sections: &Sections,
) -> Value {
    let explicit = api_key.map(SecretString::from);
    let (base, key) = match host {
        InfoHost::Featherless => {
            let (base, key) = featherless::resolve_provider_info(
                &sections.hosted_chat_settings,
                api_base,
                explicit.as_ref(),
            );
            (Some(base), key)
        }
        InfoHost::Aiml => {
            let (base, key) = aiml::resolve_provider_info(
                &sections.hosted_chat_settings,
                api_base,
                explicit.as_ref(),
            );
            (Some(base), key)
        }
        InfoHost::Empower => alias_info(
            CompatibleChatAlias::Empower,
            sections,
            api_base,
            explicit.as_ref(),
        ),
        InfoHost::Friendliai => alias_info(
            CompatibleChatAlias::Friendliai,
            sections,
            api_base,
            explicit.as_ref(),
        ),
        InfoHost::Galadriel => alias_info(
            CompatibleChatAlias::Galadriel,
            sections,
            api_base,
            explicit.as_ref(),
        ),
        InfoHost::Github => alias_info(
            CompatibleChatAlias::Github,
            sections,
            api_base,
            explicit.as_ref(),
        ),
    };
    let key = key.map(|secret| secret.expose_secret().to_owned());
    projected(json!({"base": base, "key": key}))
}

fn alias_info(
    alias: CompatibleChatAlias,
    sections: &Sections,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> (Option<String>, Option<SecretString>) {
    let (_, base, key) = alias.provider_info(&sections.openai_like_settings, api_base, api_key);
    (base, Some(key))
}

fn validate_value(api_key: Option<&str>, headers: Option<BTreeMap<String, String>>) -> Value {
    let key = api_key.map(SecretString::from);
    match featherless::validate_environment(key.as_ref(), headers) {
        Ok(validated) => projected(json!(validated)),
        Err(error) => rejected(error.status_code()),
    }
}

fn baseten_models_value(models: &[String]) -> Value {
    projected(Value::Array(
        models
            .iter()
            .map(|model| {
                json!({
                    "model": model,
                    "dedicated": baseten_endpoint::is_dedicated_deployment(model),
                    "api_base": baseten_endpoint::get_api_base_for_model(model),
                })
            })
            .collect(),
    ))
}

fn mapped_value(snapshot: Value, options: &IndexMap<String, Value>) -> Value {
    projected(json!({
        "config": snapshot,
        "options": options
            .iter()
            .map(|(name, value)| json!([name, value]))
            .collect::<Vec<Value>>(),
    }))
}

fn projected(value: Value) -> Value {
    json!({
        "ok": true,
        "value": value,
    })
}

fn rejected(status: Option<u16>) -> Value {
    json!({
        "ok": false,
        "error": {"status": status},
    })
}

fn deserialize_settings<Settings>(
    config: &BTreeMap<String, Value>,
) -> Result<Settings, Box<HelperError>>
where
    Settings: serde::de::DeserializeOwned,
{
    serde_json::from_value(Value::Object(
        config
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    ))
    .map_err(|error| helper_error(error.to_string()))
}

fn snapshot<Settings: serde::Serialize>(settings: &Settings) -> Result<Value, Box<HelperError>> {
    provider_config(settings)
        .map(Value::Object)
        .map_err(|error| helper_error(error.to_string()))
}

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
