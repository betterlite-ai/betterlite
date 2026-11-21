use serde::Deserialize;

use self::resolve::router_config;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::sections::Section;

mod resolve;
mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";

// ComplexityRouterConfig's declaration defaults for the keys this section reads
pub const DEFAULT_STALL_ENABLED: bool = false;
pub const DEFAULT_STALL_WINDOW: u32 = 6;
pub const DEFAULT_STALL_REPEAT: u32 = 3;
pub const DEFAULT_SEMANTIC_MATCHING: bool = false;
pub const DEFAULT_MATCH_THRESHOLD: f64 = 0.5;
pub const DEFAULT_HOUSEKEEPING: bool = true;

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
    keyword_tier_rules: Option<Vec<RawKeywordTierRule>>,
    #[serde(default)]
    stall_escalation_enabled: Option<RawFlag>,
    #[serde(default)]
    stall_escalation_window: Option<RawNumber>,
    #[serde(default, rename = "stall_escalation_repeat_threshold")]
    stall_repeat_threshold: Option<RawNumber>,
    #[serde(default)]
    semantic_keyword_matching: Option<RawFlag>,
    #[serde(default)]
    embedding_model: Option<String>,
    #[serde(default)]
    match_threshold: Option<RawNumber>,
    #[serde(default, rename = "route_housekeeping_to_cheapest_tier")]
    housekeeping_cheapest: Option<RawFlag>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKeywordTierRule {
    #[serde(default)]
    keywords: Option<Vec<String>>,
    #[serde(default)]
    tier: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Literal(bool),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordTierRule {
    pub keywords: Vec<String>,
    pub tier: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeywordMatching {
    #[default]
    Lexical,
    Semantic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub keyword_tier_rules: Option<Vec<KeywordTierRule>>,
    pub stall_escalation_enabled: bool,
    pub stall_escalation_window: u32,
    pub stall_repeat_threshold: u32,
    pub keyword_matching: KeywordMatching,
    pub embedding_model: Option<String>,
    pub match_threshold: f64,
    pub housekeeping_cheapest: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keyword_tier_rules: None,
            stall_escalation_enabled: DEFAULT_STALL_ENABLED,
            stall_escalation_window: DEFAULT_STALL_WINDOW,
            stall_repeat_threshold: DEFAULT_STALL_REPEAT,
            keyword_matching: KeywordMatching::Lexical,
            embedding_model: None,
            match_threshold: DEFAULT_MATCH_THRESHOLD,
            housekeeping_cheapest: DEFAULT_HOUSEKEEPING,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    // index-aligned with model_list; None when the deployment configures none of the eight keys
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
                    .map(|raw| router_config(raw, index, lookup))
                    .transpose()?,
            );
        }
        Ok(Settings { deployments })
    }
}
