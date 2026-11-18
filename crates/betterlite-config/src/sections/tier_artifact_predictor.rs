use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

pub mod artifact;
mod tests;

use self::artifact::Artifact;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";
const ARTIFACT_KEY: &str = "heuristic_v2_artifact";
const THRESHOLD_KEY: &str = "heuristic_v2_success_threshold";

// heuristic_v2_artifact defaults to the packaged ultrafeedback artifact
pub const DEFAULT_ARTIFACT: &str = "ultrafeedback";

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
    heuristic_v2_artifact: Option<RawArtifactValue>,
    heuristic_v2_success_threshold: Option<RawNumber>,
}

// heuristic_v2_artifact: TrainedTierArtifact | Literal["ultrafeedback"]
#[derive(Debug, Deserialize)]
#[serde(
    untagged,
    expecting = "a built-in artifact name or a tier artifact object"
)]
enum RawArtifactValue {
    BuiltIn(String),
    // boxed to stay within 3x of BuiltIn, the limit variant_size_differences enforces
    Inline(Box<Artifact>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArtifactSource {
    BuiltIn(String),
    Inline(Box<Artifact>),
}

impl Default for ArtifactSource {
    fn default() -> Self {
        Self::BuiltIn(DEFAULT_ARTIFACT.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub artifact: ArtifactSource,
    pub success_threshold: Option<f64>,
}

impl Config {
    // heuristic_v2_success_threshold: Field(ge=0.0, le=1.0) on the reference's config model
    pub fn validate(&self, path: &str) -> Result<(), ConfigError> {
        if self
            .success_threshold
            .is_none_or(|threshold| (0.0_f64..=1.0_f64).contains(&threshold))
        {
            return Ok(());
        }
        Err(invalid_scalar(path))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures no complexity router
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
            let configured = deployment.litellm_params.complexity_router_config;
            deployments.push(
                configured
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
        let artifact = match self.heuristic_v2_artifact {
            Some(declared) => declared.resolve(&format!("{prefix}.{ARTIFACT_KEY}"), lookup)?,
            None => ArtifactSource::default(),
        };
        let threshold_path = format!("{prefix}.{THRESHOLD_KEY}");
        if let Some(declared) = &self.heuristic_v2_success_threshold {
            reject_quoted(declared, &threshold_path)?;
        }
        let success_threshold = resolve_number(
            self.heuristic_v2_success_threshold,
            &threshold_path,
            lookup,
            parse_float,
        )?;
        let config = Config {
            artifact,
            success_threshold,
        };
        config.validate(&threshold_path)?;
        Ok(config)
    }
}

impl RawArtifactValue {
    fn resolve<L>(self, path: &str, lookup: &L) -> Result<ArtifactSource, ConfigError>
    where
        L: EnvironmentLookup,
    {
        match self {
            Self::BuiltIn(name) => {
                let resolved = resolve_string(name, path, lookup)?;
                if resolved == DEFAULT_ARTIFACT {
                    Ok(ArtifactSource::BuiltIn(resolved))
                } else {
                    Err(invalid_scalar(path))
                }
            }
            Self::Inline(artifact) => Ok(ArtifactSource::Inline(Box::new(
                artifact.resolve(path, lookup)?,
            ))),
        }
    }
}

// a declared number is a number; only an os.environ reference may be text
fn reject_quoted(value: &RawNumber, path: &str) -> Result<(), ConfigError> {
    if let RawNumber::Text(text) = value
        && !text.starts_with("os.environ/")
    {
        return Err(invalid_scalar(path));
    }
    Ok(())
}

fn invalid_scalar(path: &str) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    }
}
