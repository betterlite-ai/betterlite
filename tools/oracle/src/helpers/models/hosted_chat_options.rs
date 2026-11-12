use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::Sections;
use betterlite_models::baseten_endpoint;
use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::hosted_chat_options::baseten::{self, BasetenSettings};
use betterlite_openai::hosted_chat_options::cerebras::{self, CerebrasSettings};
use betterlite_openai::hosted_chat_options::featherless::{self, DropParams, FeatherlessSettings};
use betterlite_openai::hosted_chat_options::{CompatibleChatAlias, aiml, map_renaming_token_limit};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
mod tests;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
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
#[serde(tag = "op", rename_all = "snake_case")]
enum Call {
    Map(MapCall),
    Info(InfoCall),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapCall {
    host: MapHost,
    model: String,
    config: BTreeMap<String, Value>,
    params: Vec<(String, Value)>,
    #[serde(default)]
    drop_params: bool,
    #[serde(default)]
    supports_reasoning: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InfoCall {
    host: InfoHost,
    api_base: Option<String>,
    api_key: Option<String>,
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MapHost {
    Baseten,
    Cerebras,
    Featherless,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum InfoHost {
    Own(OwnIdentity),
    Compatible(CompatibleChatAlias),
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum OwnIdentity {
    Featherless,
    Aiml,
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
                .map(|step| match step {
                    Call::Map(call) => map(call, &sections),
                    Call::Info(call) => Ok(info(call, &sections)),
                })
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
            scenarios.push(json!({"steps": steps}));
        }
        Ok(json!(scenarios))
    }
}
fn map(call: &MapCall, sections: &Sections) -> Result<Value, Box<HelperError>> {
    match call.host {
        MapHost::Baseten => token_limit_map::<BasetenSettings>(call, baseten::SUPPORTED_PARAMS),
        MapHost::Cerebras => token_limit_map::<CerebrasSettings>(
            call,
            &cerebras::supported_params(call.supports_reasoning),
        ),
        MapHost::Featherless => {
            let mut options = IndexMap::new();
            let drop = DropParams {
                argument: call.drop_params,
                global: sections.hosted_chat_settings.drop_params,
            };
            let requested = call.params.iter().cloned().collect();
            if let Err(error) = featherless::map_openai_params(&requested, &mut options, drop) {
                return Ok(json!({"ok": false, "error": {"status": error.status_code()}}));
            }
            let snapshot = class_snapshot::<FeatherlessSettings>(&call.config)?;
            Ok(mapped_value(
                &snapshot,
                &options,
                featherless::SUPPORTED_PARAMS,
                None,
            ))
        }
    }
}
fn info(call: &InfoCall, sections: &Sections) -> Value {
    let explicit = call.api_key.clone().map(SecretString::from);
    let api_base = call.api_base.as_deref();
    let (hosted, like) = (
        &sections.hosted_chat_settings,
        &sections.openai_like_settings,
    );
    let (base, secret) = match &call.host {
        InfoHost::Own(OwnIdentity::Featherless) => {
            let (base, key) = featherless::chat_identity(hosted, api_base, explicit.as_ref());
            (Some(base), key)
        }
        InfoHost::Own(OwnIdentity::Aiml) => {
            let (base, key) = aiml::chat_identity(hosted, api_base, explicit.as_ref());
            (Some(base), key)
        }
        InfoHost::Compatible(alias) => {
            let (base, key) = alias.identity(like, api_base, explicit.as_ref());
            (base, Some(key))
        }
    };
    let key = secret.map(|key| key.expose_secret().to_owned());
    json!({"ok": true, "value": {"base": base, "key": key}})
}
fn token_limit_map<Settings>(call: &MapCall, supported: &[&str]) -> Result<Value, Box<HelperError>>
where
    Settings: serde::Serialize + serde::de::DeserializeOwned,
{
    let mut options = IndexMap::new();
    map_renaming_token_limit(
        &call.params.iter().cloned().collect(),
        &mut options,
        supported,
    );
    let baseten_model = (call.host == MapHost::Baseten).then_some(call.model.as_str());
    let snapshot = class_snapshot::<Settings>(&call.config)?;
    Ok(mapped_value(&snapshot, &options, supported, baseten_model))
}
fn mapped_value(
    snapshot: &Value,
    options: &IndexMap<String, Value>,
    supported: &[&str],
    baseten_model: Option<&str>,
) -> Value {
    let baseten = baseten_model.map(|model| {
        json!({
            "model": model,
            "dedicated": baseten_endpoint::is_dedicated_deployment(model),
            "api_base": baseten_endpoint::get_api_base_for_model(model),
        })
    });
    json!({
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
    })
}
fn class_snapshot<Settings>(config: &BTreeMap<String, Value>) -> Result<Value, Box<HelperError>>
where
    Settings: serde::Serialize + serde::de::DeserializeOwned,
{
    let object = Value::Object(config.clone().into_iter().collect());
    let settings: Settings =
        serde_json::from_value(object).map_err(|error| failure(None, error.to_string()))?;
    provider_config(&settings)
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
