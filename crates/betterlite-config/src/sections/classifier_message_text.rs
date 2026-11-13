use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawDeployment {
    #[serde(default)]
    pub litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawParams {
    #[serde(default)]
    pub complexity_router_config: Option<RawRouterConfig>,
}

// ComplexityRouterConfig is extra=allow, so the keys other sections do not read are captured here
#[derive(Debug, Default, Deserialize)]
pub struct RawRouterConfig {
    #[serde(default)]
    pub reminder_markers: Option<Vec<RawMarkerPair>>,
    #[serde(flatten)]
    pub siblings: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawMarkerPair {
    open: String,
    close: String,
    #[serde(flatten)]
    siblings: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerPair {
    pub open: String,
    pub close: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures no reminder markers
    pub deployments: Vec<Option<Vec<MarkerPair>>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let configured = deployment
                .litellm_params
                .complexity_router_config
                .and_then(|router| router.reminder_markers);
            deployments.push(
                configured
                    .map(|pairs| resolve_pairs(pairs, index, lookup))
                    .transpose()?,
            );
        }
        Ok(Settings { deployments })
    }
}

fn resolve_pairs<L>(
    pairs: Vec<RawMarkerPair>,
    index: usize,
    lookup: &L,
) -> Result<Vec<MarkerPair>, ConfigError>
where
    L: EnvironmentLookup,
{
    let prefix = format!("model_list.{index}.{PATH_ROOT}.reminder_markers");
    if pairs.is_empty() {
        return Err(ConfigError::InvalidScalar {
            path: prefix,
            value: false,
        });
    }
    let mut resolved = Vec::with_capacity(pairs.len());
    for (pair_index, pair) in pairs.into_iter().enumerate() {
        let path = format!("{prefix}.{pair_index}");
        let open = resolve_string(pair.open, &format!("{path}.open"), lookup)?
            .trim()
            .to_lowercase();
        let close = resolve_string(pair.close, &format!("{path}.close"), lookup)?
            .trim()
            .to_lowercase();
        if open.is_empty() || close.is_empty() || open == close {
            return Err(ConfigError::InvalidScalar { path, value: false });
        }
        resolved.push(MarkerPair { open, close });
    }
    Ok(resolved)
}
