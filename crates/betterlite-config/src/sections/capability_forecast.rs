use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, parse_integer, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const DEFAULT_EFFICIENT_TIER: &str = "SIMPLE";
const DEFAULT_CAPABLE_TIER: &str = "REASONING";
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 4096;
const PATH_ROOT: &str = "litellm_params.complexity_router_config.capability_classifier_config";

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
    capability_classifier_config: Option<RawCapability>,
}

#[derive(Debug, Deserialize)]
struct RawCapability {
    base_threshold: RawNumber,
    threshold_step: Option<RawNumber>,
    efficient_tier: Option<String>,
    capable_tier: Option<String>,
    max_output_tokens: Option<RawNumber>,
    calibration: Option<RawCalibration>,
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Deserialize)]
struct RawCalibration {
    version: String,
    slope: RawNumber,
    intercept: RawNumber,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    #[default]
    JsonSchema,
    JsonObject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    pub version: String,
    pub slope: f64,
    pub intercept: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub efficient_tier: String,
    pub capable_tier: String,
    pub base_threshold: f64,
    pub threshold_step: f64,
    pub max_output_tokens: u32,
    pub calibration: Option<Calibration>,
    pub response_format: ResponseFormat,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures no capability classifier
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
            let capability = deployment
                .litellm_params
                .complexity_router_config
                .and_then(|router| router.capability_classifier_config);
            deployments.push(
                capability
                    .map(|configured| configured.resolve(index, lookup))
                    .transpose()?,
            );
        }
        Ok(Settings { deployments })
    }
}

impl RawCapability {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Config, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("model_list.{index}.{PATH_ROOT}");
        let base_threshold = required_number(
            self.base_threshold,
            &format!("{prefix}.base_threshold"),
            lookup,
        )?;
        let (threshold_step, max_output_tokens) =
            limits(self.threshold_step, self.max_output_tokens, &prefix, lookup)?;
        let (efficient_tier, capable_tier) =
            tiers(self.efficient_tier, self.capable_tier, &prefix, lookup)?;
        let calibration = self
            .calibration
            .map(|calibration| calibration.resolve(&prefix, lookup))
            .transpose()?;
        Ok(Config {
            efficient_tier,
            capable_tier,
            base_threshold,
            threshold_step,
            max_output_tokens,
            calibration,
            response_format: self.response_format.unwrap_or_default(),
        })
    }
}

fn limits<L>(
    step: Option<RawNumber>,
    tokens: Option<RawNumber>,
    prefix: &str,
    lookup: &L,
) -> Result<(f64, u32), ConfigError>
where
    L: EnvironmentLookup,
{
    let threshold_step = resolve_number(
        step,
        &format!("{prefix}.threshold_step"),
        lookup,
        parse_float,
    )?
    .unwrap_or(0.0_f64);
    let max_output_tokens = resolve_number(
        tokens,
        &format!("{prefix}.max_output_tokens"),
        lookup,
        parse_integer,
    )?
    .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
    Ok((threshold_step, max_output_tokens))
}

fn tiers<L>(
    efficient: Option<String>,
    capable: Option<String>,
    prefix: &str,
    lookup: &L,
) -> Result<(String, String), ConfigError>
where
    L: EnvironmentLookup,
{
    let efficient_tier = tier(
        efficient,
        DEFAULT_EFFICIENT_TIER,
        &format!("{prefix}.efficient_tier"),
        lookup,
    )?;
    let capable_tier = tier(
        capable,
        DEFAULT_CAPABLE_TIER,
        &format!("{prefix}.capable_tier"),
        lookup,
    )?;
    Ok((efficient_tier, capable_tier))
}

fn tier<L>(
    configured: Option<String>,
    default: &str,
    path: &str,
    lookup: &L,
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    configured.map_or_else(
        || Ok(default.to_owned()),
        |text| resolve_string(text, path, lookup),
    )
}

fn required_number<L>(value: RawNumber, path: &str, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    resolve_number(Some(value), path, lookup, parse_float)?.map_or_else(
        || {
            Err(ConfigError::InvalidScalar {
                path: path.to_owned(),
                value: false,
            })
        },
        Ok,
    )
}

impl RawCalibration {
    fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<Calibration, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let path = |field: &str| format!("{prefix}.{field}");
        let version = resolve_string(self.version, &path("version"), lookup)?;
        let slope = required_number(self.slope, &path("slope"), lookup)?;
        let intercept = required_number(self.intercept, &path("intercept"), lookup)?;
        Ok(Calibration {
            version,
            slope,
            intercept,
        })
    }
}
