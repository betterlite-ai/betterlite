use std::mem::take;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

pub const DEFAULT_DOMAIN_PRIOR_MASS: f64 = 200.0;
pub const DEFAULT_COHORT_PRIOR_MASS: f64 = 20.0;
pub const DEFAULT_ROUTING_THRESHOLD: f64 = 0.75;
pub const DEFAULT_SUCCESS_DEFINITION: &str = "quality score meets the dataset success threshold";
pub const DEFAULT_SPLIT_METHOD: &str = "sha256(prompt): 70% train, 15% validation, 15% test";

// TrainedTierArtifact: the packaged JSON's wire format, and an inline heuristic_v2_artifact value
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Artifact {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub global_statistics: Vec<GlobalStatistic>,
    #[serde(default)]
    pub domain_statistics: Vec<DomainStatistic>,
    #[serde(default)]
    pub cohort_statistics: Vec<CohortStatistic>,
    #[serde(default = "default_domain_prior_mass")]
    pub domain_prior_mass: f64,
    #[serde(default = "default_cohort_prior_mass")]
    pub cohort_prior_mass: f64,
    #[serde(default = "default_routing_threshold")]
    pub routing_threshold: f64,
    #[serde(default)]
    pub datasets: Vec<Dataset>,
    #[serde(default = "default_success_definition")]
    pub success_definition: String,
    #[serde(default = "default_split_method")]
    pub split_method: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct GlobalStatistic {
    pub tier: u32,
    pub successes: f64,
    pub observations: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DomainStatistic {
    pub tier: u32,
    pub successes: f64,
    pub observations: f64,
    pub request_type: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CohortStatistic {
    pub tier: u32,
    pub successes: f64,
    pub observations: f64,
    pub cohort: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Dataset {
    pub name: String,
    pub url: String,
    pub license: String,
    pub rows: u32,
    #[serde(default = "default_success_definition")]
    pub success_definition: String,
}

impl Artifact {
    // the proxy resolves os.environ references in every text value of the config, the inline artifact's included
    pub(super) fn resolve<L>(mut self, prefix: &str, lookup: &L) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
        resolved(
            &mut self.success_definition,
            "success_definition",
            prefix,
            lookup,
        )?;
        resolved(&mut self.split_method, "split_method", prefix, lookup)?;
        for (index, statistic) in self.domain_statistics.iter_mut().enumerate() {
            let path = format!("{prefix}.domain_statistics[{index}]");
            resolved(&mut statistic.request_type, "request_type", &path, lookup)?;
        }
        for (index, statistic) in self.cohort_statistics.iter_mut().enumerate() {
            let path = format!("{prefix}.cohort_statistics[{index}]");
            resolved(&mut statistic.cohort, "cohort", &path, lookup)?;
        }
        for (index, dataset) in self.datasets.iter_mut().enumerate() {
            let path = format!("{prefix}.datasets[{index}]");
            resolved(&mut dataset.name, "name", &path, lookup)?;
            resolved(&mut dataset.url, "url", &path, lookup)?;
            resolved(&mut dataset.license, "license", &path, lookup)?;
            resolved(
                &mut dataset.success_definition,
                "success_definition",
                &path,
                lookup,
            )?;
        }
        Ok(self)
    }
}

fn resolved<L>(value: &mut String, field: &str, path: &str, lookup: &L) -> Result<(), ConfigError>
where
    L: EnvironmentLookup,
{
    *value = resolve_string(take(value), &format!("{path}.{field}"), lookup)?;
    Ok(())
}

// a literal body, since a body naming a constant equal to 1 leaves an equivalent mutant behind
const fn default_schema_version() -> u32 {
    1
}

const fn default_domain_prior_mass() -> f64 {
    DEFAULT_DOMAIN_PRIOR_MASS
}

const fn default_cohort_prior_mass() -> f64 {
    DEFAULT_COHORT_PRIOR_MASS
}

const fn default_routing_threshold() -> f64 {
    DEFAULT_ROUTING_THRESHOLD
}

fn default_split_method() -> String {
    DEFAULT_SPLIT_METHOD.to_owned()
}

// both the artifact and each dataset carry the same pinned success definition
fn default_success_definition() -> String {
    DEFAULT_SUCCESS_DEFINITION.to_owned()
}
