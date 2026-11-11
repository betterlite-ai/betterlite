use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<Deployment>,
    #[serde(default)]
    litellm_settings: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    #[serde(rename = "userContext", default)]
    user_context_camel: Option<JsonValue>,
    #[serde(rename = "user_context", default)]
    user_context_snake: Option<JsonValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserContext {
    pub camel: Option<JsonValue>,
    pub snake: Option<JsonValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub deployments: Vec<UserContext>,
    pub global: UserContext,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployments = self
            .model_list
            .into_iter()
            .enumerate()
            .map(|(index, deployment)| {
                resolve(
                    deployment.litellm_params,
                    &format!("model_list.{index}.litellm_params"),
                    lookup,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let global = resolve(self.litellm_settings, "litellm_settings", lookup)?;
        Ok(Settings {
            deployments,
            global,
        })
    }
}

fn resolve<L>(params: Params, path: &str, lookup: &L) -> Result<UserContext, ConfigError>
where
    L: EnvironmentLookup,
{
    let camel_path = format!("{path}.userContext");
    let snake_path = format!("{path}.user_context");
    let camel = params
        .user_context_camel
        .map(|value| resolve_json(value, &camel_path, lookup))
        .transpose()?;
    let snake = params
        .user_context_snake
        .map(|value| resolve_json(value, &snake_path, lookup))
        .transpose()?;
    Ok(UserContext { camel, snake })
}

fn resolve_json<L>(value: JsonValue, path: &str, lookup: &L) -> Result<JsonValue, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        JsonValue::String(text) => resolve_string(text, path, lookup).map(JsonValue::String),
        JsonValue::Object(fields) => {
            let resolved = fields
                .into_iter()
                .map(|(key, item)| {
                    resolve_json(item, &format!("{path}.{key}"), lookup)
                        .map(|resolved| (key, resolved))
                })
                .collect::<Result<_, _>>()?;
            Ok(JsonValue::Object(resolved))
        }
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)) => Ok(other),
    }
}

mod tests;
