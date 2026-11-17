use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_number;
use crate::sections::Section;

mod tests;

// AmazonTitanV2Config's class settings; the AWS defaults true and 1024 are not LiteLLM defaults
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawFields,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawFields,
}

#[derive(Debug, Default, Deserialize)]
struct RawFields {
    normalize: Option<RawNumber>,
    dimensions: Option<RawNumber>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub normalize: Option<bool>,
    pub dimensions: Option<i64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Settings::default();
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let prefix = format!("model_list.{index}.litellm_params");
            let resolved = resolve_fields(deployment.litellm_params, &prefix, lookup)?;
            deployments.normalize = resolved.normalize.or(deployments.normalize);
            deployments.dimensions = resolved.dimensions.or(deployments.dimensions);
        }
        let global = resolve_fields(self.litellm_settings, "litellm_settings", lookup)?;
        Ok(Settings {
            normalize: deployments.normalize.or(global.normalize),
            dimensions: deployments.dimensions.or(global.dimensions),
        })
    }
}

fn resolve_fields<L>(raw: RawFields, prefix: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let normalize = resolve_flag(raw.normalize, &format!("{prefix}.normalize"), lookup)?;
    let dimensions = resolve_number(
        raw.dimensions,
        &format!("{prefix}.dimensions"),
        lookup,
        |text, path| match text.trim().parse::<i64>() {
            Ok(length) => Ok(length),
            Err(source) => Err(ConfigError::InvalidInteger {
                path: path.to_owned(),
                source,
            }),
        },
    )?;
    Ok(Settings {
        normalize,
        dimensions,
    })
}

// normalize is a bool, so only an os.environ reference may arrive as text; an empty variable counts as unset
fn resolve_flag<L>(
    value: Option<RawNumber>,
    path: &str,
    lookup: &L,
) -> Result<Option<bool>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(value) = value else {
        return Ok(None);
    };
    let invalid = || ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    };
    match value {
        RawNumber::Boolean(flag) => Ok(Some(flag)),
        RawNumber::Integer(_) | RawNumber::Float(_) => Err(invalid()),
        RawNumber::Text(text) => {
            let Some(name) = text.strip_prefix("os.environ/") else {
                return Err(invalid());
            };
            let Some(resolved) = lookup.lookup(name) else {
                return Err(ConfigError::MissingEnvironment {
                    variable: name.to_owned(),
                    path: path.to_owned(),
                });
            };
            Ok(betterlite_env::flag(name, &resolved)?)
        }
    }
}
