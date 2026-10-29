use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

// AmazonStabilityConfig's class settings only; Titan, Nova, and Stability 3 never read these keys
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
    cfg_scale: Option<RawNumber>,
    seed: Option<RawNumber>,
    steps: Option<RawNumber>,
    width: Option<RawNumber>,
    height: Option<RawNumber>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub cfg_scale: Option<i64>,
    pub seed: Option<f64>,
    pub steps: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let deployment = merge_deployments(self.model_list, lookup)?;
        let global = resolve_fields(self.litellm_settings, "litellm_settings", lookup)?;
        Ok(Settings {
            cfg_scale: deployment.cfg_scale.or(global.cfg_scale),
            seed: deployment.seed.or(global.seed),
            steps: deployment.steps.or(global.steps),
            width: deployment.width.or(global.width),
            height: deployment.height.or(global.height),
        })
    }
}

// deployment litellm_params win over global litellm_settings; the last deployment in model_list wins ties
fn merge_deployments<L>(model_list: Vec<RawDeployment>, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut merged = Settings::default();
    for (index, deployment) in model_list.into_iter().enumerate() {
        let prefix = format!("model_list.{index}.litellm_params");
        let resolved = resolve_fields(deployment.litellm_params, &prefix, lookup)?;
        merged.cfg_scale = resolved.cfg_scale.or(merged.cfg_scale);
        merged.seed = resolved.seed.or(merged.seed);
        merged.steps = resolved.steps.or(merged.steps);
        merged.width = resolved.width.or(merged.width);
        merged.height = resolved.height.or(merged.height);
    }
    Ok(merged)
}

fn resolve_fields<L>(raw: RawFields, prefix: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let cfg_scale = resolve_number(
        raw.cfg_scale,
        &format!("{prefix}.cfg_scale"),
        lookup,
        parse_signed_integer,
    )?;
    let seed = resolve_number(raw.seed, &format!("{prefix}.seed"), lookup, parse_float)?;
    let steps = resolve_number(
        raw.steps,
        &format!("{prefix}.steps"),
        lookup,
        parse_signed_integer,
    )?;
    let width = resolve_number(
        raw.width,
        &format!("{prefix}.width"),
        lookup,
        parse_signed_integer,
    )?;
    let height = resolve_number(
        raw.height,
        &format!("{prefix}.height"),
        lookup,
        parse_signed_integer,
    )?;
    Ok(Settings {
        cfg_scale,
        seed,
        steps,
        width,
        height,
    })
}

fn parse_signed_integer(text: &str, path: &str) -> Result<i64, ConfigError> {
    let trimmed = text.trim();
    trimmed
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

mod tests;
