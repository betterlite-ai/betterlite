use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::Sections;
use betterlite_models::baseten_endpoint;
use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::hosted_chat_options::baseten::{self, BasetenSettings};
use betterlite_openai::hosted_chat_options::cerebras::{self, CerebrasSettings};
use betterlite_openai::hosted_chat_options::featherless::{self, DropParams, FeatherlessSettings};
use betterlite_openai::hosted_chat_options::identity;
use betterlite_openai::hosted_chat_options::map_renaming_token_limit;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
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
#[serde(deny_unknown_fields)]
struct Call {
    op: Kind,
    #[serde(default)]
    host: Option<Host>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    config: BTreeMap<String, Value>,
    #[serde(default)]
    params: Vec<(String, Value)>,
    #[serde(default)]
    drop_params: bool,
    #[serde(default)]
    supports_reasoning: bool,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
}
#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Map,
    Info,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Host {
    Baseten,
    Cerebras,
    Featherless,
    Aiml,
    Empower,
    Friendliai,
    Galadriel,
    Github,
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let mut scenarios = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            let sections = ConfigLoader {
                lookup: scenario.environment.clone(),
            }
            .load("")
            .map_err(|error| failure(None, error.to_string()))?
            .sections;
            let steps = scenario
                .calls
                .iter()
                .map(|call| step(call, &sections))
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
            scenarios.push(json!({"steps": steps}));
        }
        Ok(json!(scenarios))
    }
}
fn step(call: &Call, sections: &Sections) -> Result<Value, Box<HelperError>> {
    match call.op {
        Kind::Map => map(call, sections),
        Kind::Info => info(call, sections),
    }
}
fn map(call: &Call, sections: &Sections) -> Result<Value, Box<HelperError>> {
    match call.host {
        Some(Host::Baseten) => token_limit_map::<BasetenSettings>(
            call,
            baseten::SUPPORTED_PARAMS,
            Some(call.model.as_str()),
        ),
        Some(Host::Cerebras) => token_limit_map::<CerebrasSettings>(
            call,
            &cerebras::supported_params(call.supports_reasoning),
            None,
        ),
        Some(Host::Featherless) => featherless_map(call, sections),
        Some(Host::Aiml | Host::Empower | Host::Friendliai | Host::Galadriel | Host::Github)
        | None => Err(failure(None, String::from("the host has no option map"))),
    }
}
fn info(call: &Call, sections: &Sections) -> Result<Value, Box<HelperError>> {
    let explicit = call.api_key.as_ref().map(SecretString::from);
    let (base, key) = match call.host {
        Some(Host::Featherless) => {
            let (base, key) = identity::featherless(
                &sections.hosted_chat_settings,
                call.api_base.as_deref(),
                explicit.as_ref(),
            );
            (Some(base), key)
        }
        Some(Host::Aiml) => {
            let (base, key) = identity::aiml(
                &sections.hosted_chat_settings,
                call.api_base.as_deref(),
                explicit.as_ref(),
            );
            (Some(base), key)
        }
        Some(Host::Empower | Host::Friendliai | Host::Galadriel | Host::Github) => {
            let (base, key) = sections
                .openai_like_settings
                .resolve(call.api_base.as_deref(), explicit.as_ref());
            (base, Some(key))
        }
        Some(Host::Baseten | Host::Cerebras) | None => {
            return Err(failure(None, String::from("the host has no identity")));
        }
    };
    let key = key.map(|secret| secret.expose_secret().to_owned());
    Ok(json!({"ok": true, "value": {"base": base, "key": key}}))
}
fn token_limit_map<Settings>(
    call: &Call,
    supported: &[&str],
    baseten_model: Option<&str>,
) -> Result<Value, Box<HelperError>>
where
    Settings: serde::Serialize + serde::de::DeserializeOwned,
{
    let settings: Settings = deserialize_settings(&call.config)?;
    let mut options = IndexMap::new();
    map_renaming_token_limit(
        &call.params.iter().cloned().collect(),
        &mut options,
        supported,
    );
    mapped_value(snapshot(&settings)?, &options, supported, baseten_model)
}
fn featherless_map(call: &Call, sections: &Sections) -> Result<Value, Box<HelperError>> {
    let settings: FeatherlessSettings = deserialize_settings(&call.config)?;
    let mut options = IndexMap::new();
    let drop = DropParams {
        argument: call.drop_params,
        global: sections.hosted_chat_settings.drop_params,
    };
    match featherless::map_openai_params(&call.params.iter().cloned().collect(), &mut options, drop)
    {
        Ok(()) => mapped_value(
            snapshot(&settings)?,
            &options,
            featherless::SUPPORTED_PARAMS,
            None,
        ),
        Err(error) => Ok(json!({"ok": false, "error": {"status": error.status_code()}})),
    }
}
fn mapped_value(
    snapshot: Value,
    options: &IndexMap<String, Value>,
    supported: &[&str],
    baseten_model: Option<&str>,
) -> Result<Value, Box<HelperError>> {
    let baseten = baseten_model.map(|model| {
        json!({
            "model": model,
            "dedicated": baseten_endpoint::is_dedicated_deployment(model),
            "api_base": baseten_endpoint::get_api_base_for_model(model),
        })
    });
    Ok(json!({
        "ok": true,
        "value": {
            "config": snapshot,
            "supported": supported,
            "options": options
                .iter()
                .map(|(name, item)| json!([name, item]))
                .collect::<Vec<Value>>(),
            "baseten": baseten,
        },
    }))
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
    .map_err(|error| failure(None, error.to_string()))
}
fn snapshot<Settings: serde::Serialize>(settings: &Settings) -> Result<Value, Box<HelperError>> {
    provider_config(settings)
        .map(Value::Object)
        .map_err(|error| failure(None, error.to_string()))
}
fn failure(status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
