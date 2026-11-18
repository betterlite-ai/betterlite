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
    #[error("custom dimension pattern {pattern:?} {}", problem.message())]
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
    let mut characters = config.name.chars();
    let first = characters.next();
    if !first.is_some_and(|first| first.is_ascii_alphabetic())
        || config.name.chars().count() > MAX_NAME_CHARS
        || !characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(DimensionProblem::Name);
    }
    if !(config.weight.is_finite() && config.weight > 0.0 && config.weight <= 1.0) {
        return Err(DimensionProblem::Weight);
    }
    let matchers = config.keywords.iter().chain(&config.patterns);
    if matchers
        .clone()
        .any(|item| !(1..=MAX_MATCHER_CHARS).contains(&item.chars().count()))
    {
        return Err(DimensionProblem::MatcherLength);
    }
    let scoring_mode = match config.scoring_mode.as_str() {
        "binary" => ScoringMode::Binary,
        "match_count" => ScoringMode::MatchCount,
        _ => return Err(DimensionProblem::ScoringMode),
    };
    if config.keywords.is_empty() && config.patterns.is_empty() {
        return Err(DimensionProblem::BlankMatchers);
    }
    if matchers.clone().any(|item| {
        item.chars()
            .all(|character| character.is_whitespace() || matches!(character, '\u{1C}'..='\u{1F}'))
    }) {
        return Err(DimensionProblem::BlankMatchers);
    }
    let count = config.keywords.len().saturating_add(config.patterns.len());
    let total_chars = matchers.map(|item| item.chars().count()).sum::<usize>();
    if count > MAX_MATCHERS || total_chars > MAX_MATCHER_TOTAL_CHARS {
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

fn default_scoring_mode() -> String {
    "binary".to_owned()
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
            for (position, raw) in configured.into_iter().enumerate() {
                let path = format!("model_list.{index}.{PATH_ROOT}.{position}");
                let name = resolve_string(
                    raw.name.unwrap_or_default(),
                    &format!("{path}.name"),
                    lookup,
                )?;
                let weight = match raw.weight {
                    Some(RawNumber::Text(text)) if !text.starts_with("os.environ/") => {
                        return Err(ConfigError::InvalidScalar {
                            path: format!("{path}.weight"),
                            value: false,
                        });
                    }
                    weight => {
                        resolve_number(weight, &format!("{path}.weight"), lookup, parse_float)?
                            .unwrap_or(0.0)
                    }
                };
                let mut keywords = Vec::with_capacity(raw.keywords.len());
                for (matcher, keyword) in raw.keywords.into_iter().enumerate() {
                    keywords.push(resolve_string(
                        keyword,
                        &format!("{path}.keywords.{matcher}"),
                        lookup,
                    )?);
                }
                let mut patterns = Vec::with_capacity(raw.patterns.len());
                for (matcher, pattern) in raw.patterns.into_iter().enumerate() {
                    patterns.push(resolve_string(
                        pattern,
                        &format!("{path}.patterns.{matcher}"),
                        lookup,
                    )?);
                }
                let scoring_mode = resolve_string(
                    raw.scoring_mode.unwrap_or_else(default_scoring_mode),
                    &format!("{path}.scoring_mode"),
                    lookup,
                )?;
                dimensions.push(Config {
                    name,
                    weight,
                    keywords,
                    patterns,
                    scoring_mode,
                });
            }
            deployments.push(dimensions);
        }
        Ok(Settings { deployments })
    }
}
