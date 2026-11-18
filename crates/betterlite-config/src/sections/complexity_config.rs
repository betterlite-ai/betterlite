use serde::{Deserialize, Serialize};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(default)]
    complexity_router_config: Option<RawRouterConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouterConfig {
    #[serde(default)]
    escalation_keywords: Option<Vec<String>>,
    #[serde(default)]
    plan_mode_min_tier: Option<String>,
    #[serde(default)]
    plan_mode_patterns: Option<Vec<String>>,
    #[serde(default)]
    housekeeping_patterns: Option<Vec<String>>,
    #[serde(default)]
    heuristic_first_max_tier: Option<String>,
    #[serde(default)]
    hybrid_boundary_margin: Option<RawNumber>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub escalation_keywords: Option<Vec<String>>,
    #[serde(default)]
    pub plan_mode_min_tier: Option<String>,
    #[serde(default)]
    pub plan_mode_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub housekeeping_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub heuristic_first_max_tier: Option<String>,
    #[serde(default)]
    pub hybrid_boundary_margin: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures none of these keys
    pub deployments: Vec<Option<Config>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(
                deployment
                    .litellm_params
                    .complexity_router_config
                    .map(|raw| raw.resolve(index, lookup))
                    .transpose()?,
            );
        }
        Ok(Settings { deployments })
    }
}

impl RawRouterConfig {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Config, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("model_list.{index}.{PATH_ROOT}");
        Ok(Config {
            // the proxy substitutes mapping values only, so a list entry stays literal as written
            escalation_keywords: self.escalation_keywords,
            plan_mode_min_tier: optional_text(
                self.plan_mode_min_tier,
                &format!("{prefix}.plan_mode_min_tier"),
                lookup,
            )?,
            plan_mode_patterns: self.plan_mode_patterns,
            housekeeping_patterns: self.housekeeping_patterns,
            heuristic_first_max_tier: optional_text(
                self.heuristic_first_max_tier,
                &format!("{prefix}.heuristic_first_max_tier"),
                lookup,
            )?,
            hybrid_boundary_margin: optional_number(
                self.hybrid_boundary_margin,
                &format!("{prefix}.hybrid_boundary_margin"),
                lookup,
            )?,
        })
    }
}

// an os.environ/ reference resolves once here, and an empty variable counts as unset
fn optional_text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let is_reference = text.starts_with("os.environ/");
    let resolved = resolve_string(text, path, lookup)?;
    if is_reference && resolved.is_empty() {
        return Ok(None);
    }
    Ok(Some(resolved))
}

fn optional_number<L>(
    value: Option<RawNumber>,
    path: &str,
    lookup: &L,
) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(declared) = &value {
        reject_quoted(declared, path)?;
    }
    resolve_number(value, path, lookup, parse_float)
}

// a margin is a declared number; only an os.environ reference may be text
fn reject_quoted(value: &RawNumber, path: &str) -> Result<(), ConfigError> {
    if let RawNumber::Text(text) = value
        && !text.starts_with("os.environ/")
    {
        return Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        });
    }
    Ok(())
}
