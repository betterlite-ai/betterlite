use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::Sections;
use betterlite_models::baseten_endpoint;
use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::hosted_chat_options::baseten::{self, BasetenSettings};
use betterlite_openai::hosted_chat_options::cerebras::{self, CerebrasSettings};
use betterlite_openai::hosted_chat_options::featherless::{
    self, ChatIdentity, DropParams, FeatherlessSettings,
};
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
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Call {
    op: Kind,
    host: String,
    model: String,
    config: BTreeMap<String, Value>,
    params: Vec<(String, Value)>,
    drop_params: bool,
    supports_reasoning: bool,
    api_base: Option<String>,
    api_key: Option<String>,
}
#[derive(Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
enum Kind {
    #[default]
    Map,
    Info,
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
                .map(|call| match call.op {
                    Kind::Map => map(call, &sections),
                    Kind::Info => info(call, &sections),
                })
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
            scenarios.push(json!({"steps": steps}));
        }
        Ok(json!(scenarios))
    }
}
fn map(call: &Call, sections: &Sections) -> Result<Value, Box<HelperError>> {
    match call.host.as_str() {
        "baseten" => token_limit_map::<BasetenSettings>(call, baseten::SUPPORTED_PARAMS),
        "cerebras" => token_limit_map::<CerebrasSettings>(
            call,
            &cerebras::supported_params(call.supports_reasoning),
        ),
        "featherless" => {
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
                snapshot,
                &options,
                featherless::SUPPORTED_PARAMS,
                None,
            ))
        }
        _ => Err(failure(None, String::from("the host has no option map"))),
    }
}
fn info(call: &Call, sections: &Sections) -> Result<Value, Box<HelperError>> {
    let explicit = call.api_key.as_ref().map(SecretString::from);
    let chat_host = match call.host.as_str() {
        "featherless" => Some(ChatIdentity::Featherless),
        "aiml" => Some(ChatIdentity::Aiml),
        "empower" | "friendliai" | "galadriel" | "github" => None,
        _ => return Err(failure(None, String::from("the host has no identity"))),
    };
    let (base, key) = if let Some(host) = chat_host {
        let hosted = &sections.hosted_chat_settings;
        let (base, key) =
            featherless::chat_identity(host, hosted, call.api_base.as_deref(), explicit.as_ref());
        (Some(base), key)
    } else {
        let (base, key) = sections
            .openai_like_settings
            .resolve(call.api_base.as_deref(), explicit.as_ref());
        (base, Some(key))
    };
    let key = key.map(|secret| secret.expose_secret().to_owned());
    Ok(json!({"ok": true, "value": {"base": base, "key": key}}))
}
fn token_limit_map<Settings>(call: &Call, supported: &[&str]) -> Result<Value, Box<HelperError>>
where
    Settings: serde::Serialize + serde::de::DeserializeOwned,
{
    let mut options = IndexMap::new();
    map_renaming_token_limit(
        &call.params.iter().cloned().collect(),
        &mut options,
        supported,
    );
    let baseten_model = (call.host == "baseten").then_some(call.model.as_str());
    let snapshot = class_snapshot::<Settings>(&call.config)?;
    Ok(mapped_value(snapshot, &options, supported, baseten_model))
}
fn mapped_value(
    snapshot: Value,
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
