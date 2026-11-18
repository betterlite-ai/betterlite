use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

mod pattern;
mod tests;

pub use pattern::{PatternProblem, pattern_work};

const PATH_ROOT: &str = "litellm_params.complexity_router_config.custom_dimensions";
const MAX_NAME_CHARS: usize = 64;
const MAX_MATCHER_CHARS: usize = 256;
const MAX_MATCHERS: usize = 32;
const MAX_MATCHER_TOTAL_CHARS: usize = 4096;

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
pub struct RawRouterConfig {
    #[serde(default)]
    custom_dimensions: Option<Vec<RawDimension>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDimension {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    weight: Option<RawNumber>,
    #[serde(default)]
    keywords: Option<Vec<String>>,
    #[serde(default)]
    patterns: Option<Vec<String>>,
    #[serde(default)]
    scoring_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub weight: f64,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default = "default_scoring_mode")]
    pub scoring_mode: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringMode {
    Binary,
    MatchCount,
}

impl ScoringMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "binary" => Some(Self::Binary),
            "match_count" => Some(Self::MatchCount),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub name: String,
    pub weight: f64,
    pub keywords: Vec<String>,
    pub patterns: Vec<String>,
    pub scoring_mode: ScoringMode,
    pub work: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DimensionProblem {
    #[error(
        "a custom dimension name must be 1 to 64 ASCII letters, digits or underscores and start with a letter"
    )]
    Name,
    #[error("a custom dimension weight must be finite, greater than 0 and at most 1")]
    Weight,
    #[error("custom dimension keywords and patterns must be 1 to 256 characters")]
    MatcherLength,
    #[error("custom dimension scoring_mode must be binary or match_count")]
    ScoringMode,
    #[error("custom dimensions require nonblank keywords and/or patterns")]
    BlankMatchers,
    #[error("custom dimensions allow at most 32 matchers and 4096 matcher characters each")]
    MatcherLimits,
    #[error("custom dimension pattern {pattern:?} {problem}")]
    Pattern {
        pattern: String,
        problem: PatternProblem,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub deployments: Vec<Vec<Config>>,
}

pub fn validate_dimension(config: &Config) -> Result<Checked, DimensionProblem> {
    if !valid_name(&config.name) {
        return Err(DimensionProblem::Name);
    }
    if !valid_weight(config.weight) {
        return Err(DimensionProblem::Weight);
    }
    if config
        .keywords
        .iter()
        .chain(&config.patterns)
        .any(|item| !valid_matcher_length(item))
    {
        return Err(DimensionProblem::MatcherLength);
    }
    let scoring_mode =
        ScoringMode::parse(&config.scoring_mode).ok_or(DimensionProblem::ScoringMode)?;
    if config.keywords.is_empty() && config.patterns.is_empty() {
        return Err(DimensionProblem::BlankMatchers);
    }
    let matchers = config.keywords.iter().chain(&config.patterns);
    if matchers.clone().any(|item| blank(item)) {
        return Err(DimensionProblem::BlankMatchers);
    }
    let count = config.keywords.len().saturating_add(config.patterns.len());
    let characters = config
        .keywords
        .iter()
        .chain(&config.patterns)
        .map(|item| item.chars().count())
        .fold(0_usize, usize::saturating_add);
    if count > MAX_MATCHERS || characters > MAX_MATCHER_TOTAL_CHARS {
        return Err(DimensionProblem::MatcherLimits);
    }
    let mut work = 0_u64;
    for pattern in &config.patterns {
        match pattern_work(pattern) {
            Ok(steps) => work = work.saturating_add(steps),
            Err(problem) => {
                return Err(DimensionProblem::Pattern {
                    pattern: pattern.clone(),
                    problem,
                });
            }
        }
    }
    Ok(Checked {
        name: config.name.clone(),
        weight: config.weight,
        keywords: config.keywords.clone(),
        patterns: config.patterns.clone(),
        scoring_mode,
        work,
    })
}

fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() || name.chars().count() > MAX_NAME_CHARS {
        return false;
    }
    characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn valid_weight(weight: f64) -> bool {
    weight.is_finite() && weight > 0.0 && weight <= 1.0
}

fn valid_matcher_length(item: &str) -> bool {
    let length = item.chars().count();
    length >= 1 && length <= MAX_MATCHER_CHARS
}

fn blank(item: &str) -> bool {
    item.chars().all(python_space)
}

fn python_space(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{1C}'..='\u{1F}')
}

fn default_scoring_mode() -> String {
    "binary".to_owned()
}

impl RawDimension {
    fn resolve<L>(self, path: &str, lookup: &L) -> Result<Config, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Config {
            name: resolve_string(
                self.name.unwrap_or_default(),
                &format!("{path}.name"),
                lookup,
            )?,
            weight: resolve_weight(self.weight, &format!("{path}.weight"), lookup)?,
            keywords: resolve_texts(
                self.keywords.unwrap_or_default(),
                &format!("{path}.keywords"),
                lookup,
            )?,
            patterns: resolve_texts(
                self.patterns.unwrap_or_default(),
                &format!("{path}.patterns"),
                lookup,
            )?,
            scoring_mode: resolve_string(
                self.scoring_mode.unwrap_or_else(default_scoring_mode),
                &format!("{path}.scoring_mode"),
                lookup,
            )?,
        })
    }
}

fn resolve_weight<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(RawNumber::Text(text)) = &value
        && !text.starts_with("os.environ/")
    {
        return Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        });
    }
    Ok(resolve_number(value, path, lookup, parse_float)?.unwrap_or(0.0))
}

fn resolve_texts<L>(values: Vec<String>, path: &str, lookup: &L) -> Result<Vec<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = Vec::with_capacity(values.len());
    for (index, value) in values.into_iter().enumerate() {
        resolved.push(resolve_string(value, &format!("{path}.{index}"), lookup)?);
    }
    Ok(resolved)
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
                .and_then(|config| config.custom_dimensions)
                .unwrap_or_default();
            let mut dimensions = Vec::with_capacity(configured.len());
            for (position, dimension) in configured.into_iter().enumerate() {
                let path = format!("model_list.{index}.{PATH_ROOT}.{position}");
                dimensions.push(dimension.resolve(&path, lookup)?);
            }
            deployments.push(dimensions);
        }
        Ok(Settings { deployments })
    }
}
