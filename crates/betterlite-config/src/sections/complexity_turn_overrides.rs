use serde::Deserialize;

use self::resolve::{keyword_rules, resolve_flag, semantic, stall};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::sections::Section;

mod resolve;
mod tests;

const PATH_ROOT: &str = "litellm_params.complexity_router_config";

// ComplexityRouterConfig's declaration defaults for the keys this section reads
pub const DEFAULT_STALL_ESCALATION_ENABLED: bool = false;
pub const DEFAULT_STALL_ESCALATION_WINDOW: u32 = 6;
pub const DEFAULT_STALL_ESCALATION_REPEAT_THRESHOLD: u32 = 3;
pub const DEFAULT_SEMANTIC_KEYWORD_MATCHING: bool = false;
pub const DEFAULT_MATCH_THRESHOLD: f64 = 0.5;
pub const DEFAULT_ROUTE_HOUSEKEEPING_TO_CHEAPEST_TIER: bool = true;

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
    #[serde(default)]
    stall_escalation_repeat_threshold: Option<RawNumber>,
    #[serde(default)]
    semantic_keyword_matching: Option<RawFlag>,
    #[serde(default)]
    embedding_model: Option<String>,
    #[serde(default)]
    match_threshold: Option<RawNumber>,
    #[serde(default)]
    route_housekeeping_to_cheapest_tier: Option<RawFlag>,
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
    pub stall_escalation_repeat_threshold: u32,
    pub keyword_matching: KeywordMatching,
    pub embedding_model: Option<String>,
    pub match_threshold: f64,
    pub route_housekeeping_to_cheapest_tier: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keyword_tier_rules: None,
            stall_escalation_enabled: DEFAULT_STALL_ESCALATION_ENABLED,
            stall_escalation_window: DEFAULT_STALL_ESCALATION_WINDOW,
            stall_escalation_repeat_threshold: DEFAULT_STALL_ESCALATION_REPEAT_THRESHOLD,
            keyword_matching: KeywordMatching::Lexical,
            embedding_model: None,
            match_threshold: DEFAULT_MATCH_THRESHOLD,
            route_housekeeping_to_cheapest_tier: DEFAULT_ROUTE_HOUSEKEEPING_TO_CHEAPEST_TIER,
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
        let keyword_tier_rules = keyword_rules(self.keyword_tier_rules, &prefix)?;
        let (stall_enabled, stall_window, stall_repeat) = stall(
            self.stall_escalation_enabled,
            self.stall_escalation_window,
            self.stall_escalation_repeat_threshold,
            &prefix,
            lookup,
        )?;
        let (keyword_matching, embedding_model, match_threshold) = semantic(
            self.semantic_keyword_matching,
            self.embedding_model,
            self.match_threshold,
            &prefix,
            lookup,
        )?;
        Ok(Config {
            keyword_tier_rules,
            stall_escalation_enabled: stall_enabled,
            stall_escalation_window: stall_window,
            stall_escalation_repeat_threshold: stall_repeat,
            keyword_matching,
            embedding_model,
            match_threshold,
            route_housekeeping_to_cheapest_tier: resolve_flag(
                self.route_housekeeping_to_cheapest_tier,
                &format!("{prefix}.route_housekeeping_to_cheapest_tier"),
                lookup,
            )?
            .unwrap_or(DEFAULT_ROUTE_HOUSEKEEPING_TO_CHEAPEST_TIER),
        })
    }
}
