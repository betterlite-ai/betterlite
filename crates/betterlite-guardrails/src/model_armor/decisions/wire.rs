use indexmap::IndexMap;
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

// The fields of a sanitizeUserPrompt / sanitizeModelResponse body the verdict reads; the raw body stays a GuardrailValue for logging.
#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SanitizeResponse {
    #[serde_as(as = "DefaultOnNull")]
    pub sanitization_result: SanitizationResult,
    #[serde_as(as = "DefaultOnNull")]
    pub sanitized_text: String,
    #[serde_as(as = "DefaultOnNull")]
    pub text: String,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SanitizationResult {
    #[serde_as(as = "DefaultOnNull")]
    pub filter_results: FilterResults,
}

// Most templates name each filter in a map, some send a plain list.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum FilterResults {
    Named(IndexMap<String, FilterEntry>),
    Listed(Vec<FilterEntry>),
}

impl Default for FilterResults {
    fn default() -> Self {
        Self::Named(IndexMap::new())
    }
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FilterEntry {
    #[serde(rename = "raiFilterResult")]
    #[serde_as(as = "DefaultOnNull")]
    pub rai: MatchResult,
    #[serde(rename = "piAndJailbreakFilterResult")]
    #[serde_as(as = "DefaultOnNull")]
    pub pi_and_jailbreak: MatchResult,
    #[serde(rename = "maliciousUriFilterResult")]
    #[serde_as(as = "DefaultOnNull")]
    pub malicious_uri: MatchResult,
    #[serde(rename = "csamFilterFilterResult")]
    #[serde_as(as = "DefaultOnNull")]
    pub csam: MatchResult,
    #[serde(rename = "virusScanFilterResult")]
    #[serde_as(as = "DefaultOnNull")]
    pub virus_scan: MatchResult,
    #[serde(rename = "sdpFilterResult")]
    #[serde_as(as = "DefaultOnNull")]
    pub sdp: SdpFilterResult,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SdpFilterResult {
    #[serde_as(as = "DefaultOnNull")]
    pub inspect_result: MatchResult,
    #[serde_as(as = "DefaultOnNull")]
    pub deidentify_result: DeidentifyResult,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MatchResult {
    #[serde_as(as = "DefaultOnNull")]
    pub match_state: MatchState,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DeidentifyResult {
    #[serde_as(as = "DefaultOnNull")]
    pub match_state: MatchState,
    #[serde_as(as = "DefaultOnNull")]
    pub data: DeidentifyData,
}

#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DeidentifyData {
    #[serde_as(as = "DefaultOnNull")]
    pub text: String,
}

// LiteLLM compares the state text exactly, so any other spelling, case included, is not a match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MatchState {
    #[default]
    #[serde(rename = "MATCH_STATE_UNSPECIFIED")]
    Unspecified,
    NoMatchFound,
    MatchFound,
    #[serde(other)]
    Unrecognized,
}
