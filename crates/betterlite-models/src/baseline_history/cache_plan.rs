use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountedBreakpoint {
    pub fingerprint: String,
    pub ttl_seconds: i64,
    pub prefix_tokens: i64,
    pub lookback_fingerprints: Vec<String>,
    #[serde(default)]
    pub content_fingerprint: String,
    #[serde(default)]
    pub lookback_content_fingerprints: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountedPromptCachePlan {
    pub total_tokens: i64,
    pub breakpoints: Vec<CountedBreakpoint>,
}

mod tests;
