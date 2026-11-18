mod tests;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

const KEY: &str = "bedrock_neutralize_orphaned_tool_blocks";

// the flags live behind a box so Sections stays small, since betterlite-server holds a Config across an await
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub flags: Box<FlagSettings>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagSettings {
    pub deployments: Vec<Option<bool>>,
    pub litellm_settings: Option<bool>,
}

impl Settings {
    #[must_use]
    pub fn resolve(&self, index: usize) -> bool {
        self.flags
            .deployments
            .get(index)
            .copied()
            .flatten()
            .or(self.flags.litellm_settings)
            .unwrap_or(true)
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawValues,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawValues,
}

#[derive(Debug, Default, Deserialize)]
struct RawValues {
    #[serde(rename = "bedrock_neutralize_orphaned_tool_blocks", default)]
    neutralize_orphaned_blocks: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let path = format!("model_list.{index}.litellm_params.{KEY}");
            deployments.push(configured_flag(
                deployment.litellm_params.neutralize_orphaned_blocks,
                &path,
                lookup,
            )?);
        }
        Ok(Settings {
            flags: Box::new(FlagSettings {
                deployments,
                litellm_settings: configured_flag(
                    self.litellm_settings.neutralize_orphaned_blocks,
                    &format!("litellm_settings.{KEY}"),
                    lookup,
                )?,
            }),
        })
    }
}

// An empty environment value counts as unset, so the global setting or the true default still applies.
fn configured_flag<L>(
    value: Option<RawFlag>,
    path: &str,
    lookup: &L,
) -> Result<Option<bool>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(None),
        Some(RawFlag::Boolean(flag)) => Ok(Some(flag)),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix("os.environ/").unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?)
        }
    }
}
