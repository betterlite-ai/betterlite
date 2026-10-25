use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

pub type BedrockGuardrailQualifier = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BedrockExtensionValue {
    Null,
    Bool(bool),
    Number(i64),
    Float(f64),
    String(String),
    Array(Vec<BedrockExtensionValue>),
    Object(BTreeMap<String, BedrockExtensionValue>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BedrockTextContent {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub qualifiers: Option<Vec<BedrockGuardrailQualifier>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BedrockContentItem {
    #[serde(default)]
    pub text: Option<BedrockTextContent>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BedrockRequest {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub content: Option<Vec<BedrockContentItem>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BedrockGuardrailUsage {
    #[serde(rename = "topicPolicyUnits", default)]
    pub topic_policy_units: Option<i64>,
    #[serde(rename = "contentPolicyUnits", default)]
    pub content_policy_units: Option<i64>,
    #[serde(rename = "wordPolicyUnits", default)]
    pub word_policy_units: Option<i64>,
    #[serde(rename = "sensitiveInformationPolicyUnits", default)]
    pub sensitive_information_policy_units: Option<i64>,
    #[serde(rename = "sensitiveInformationPolicyFreeUnits", default)]
    pub sensitive_information_policy_free_units: Option<i64>,
    #[serde(rename = "contextualGroundingPolicyUnits", default)]
    pub contextual_grounding_policy_units: Option<i64>,
    #[serde(rename = "contentPolicyImageUnits", default)]
    pub content_policy_image_units: Option<i64>,
    #[serde(rename = "automatedReasoningPolicyUnits", default)]
    pub automated_reasoning_policy_units: Option<i64>,
    #[serde(rename = "automatedReasoningPolicies", default)]
    pub automated_reasoning_policies: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, BedrockExtensionValue>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BedrockGuardrailOutput {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, BedrockExtensionValue>,
}

macro_rules! record { ($name:ident { $($field:ident: $ty:ty => $wire:literal),* $(,)? }) => {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
    pub struct $name { $(#[serde(rename=$wire, default, skip_serializing_if = "Option::is_none")] pub $field: Option<$ty>,)* #[serde(flatten)] pub extra: BTreeMap<String, BedrockExtensionValue> }
}; }
record!(BedrockGuardrailTopicPolicyItem { name: String => "name", type_name: String => "type", action: String => "action" });
record!(BedrockGuardrailTopicPolicy { topics: Vec<BedrockGuardrailTopicPolicyItem> => "topics" });
record!(BedrockGuardrailContentPolicyFilter { type_name: String => "type", confidence: String => "confidence", filter_strength: String => "filterStrength", action: String => "action" });
record!(BedrockGuardrailContentPolicy { filters: Vec<BedrockGuardrailContentPolicyFilter> => "filters" });
record!(BedrockGuardrailWordPolicyCustomWord { match_text: String => "match", action: String => "action" });
record!(BedrockGuardrailWordPolicyManagedWord { match_text: String => "match", type_name: String => "type", action: String => "action" });
record!(BedrockGuardrailWordPolicy { custom_words: Vec<BedrockGuardrailWordPolicyCustomWord> => "customWords", managed_word_lists: Vec<BedrockGuardrailWordPolicyManagedWord> => "managedWordLists" });
record!(BedrockGuardrailPiiEntity { type_name: String => "type", match_text: String => "match", action: String => "action" });
record!(BedrockGuardrailRegex { name: String => "name", regex: String => "regex", match_text: String => "match", action: String => "action" });
record!(BedrockGuardrailSensitiveInformationPolicy { pii_entities: Vec<BedrockGuardrailPiiEntity> => "piiEntities", regexes: Vec<BedrockGuardrailRegex> => "regexes" });
record!(BedrockGuardrailContextualGroundingFilter { type_name: String => "type", threshold: f64 => "threshold", score: f64 => "score", action: String => "action" });
record!(BedrockGuardrailContextualGroundingPolicy { filters: Vec<BedrockGuardrailContextualGroundingFilter> => "filters" });
record!(BedrockGuardrailCoverage { text_characters: BTreeMap<String, i64> => "textCharacters" });
record!(BedrockGuardrailInvocationMetrics { guardrail_processing_latency: i64 => "guardrailProcessingLatency", usage: BedrockGuardrailUsage => "usage", guardrail_coverage: BedrockGuardrailCoverage => "guardrailCoverage" });
record!(BedrockGuardrailAssessment { topic_policy: BedrockGuardrailTopicPolicy => "topicPolicy", content_policy: BedrockGuardrailContentPolicy => "contentPolicy", word_policy: BedrockGuardrailWordPolicy => "wordPolicy", sensitive_information_policy: BedrockGuardrailSensitiveInformationPolicy => "sensitiveInformationPolicy", contextual_grounding_policy: BedrockGuardrailContextualGroundingPolicy => "contextualGroundingPolicy", invocation_metrics: BedrockGuardrailInvocationMetrics => "invocationMetrics", guardrail_coverage: BedrockGuardrailCoverage => "guardrailCoverage" });
record!(BedrockGuardrailResponse { usage: BedrockGuardrailUsage => "usage", action: String => "action", output: Vec<BedrockGuardrailOutput> => "output", outputs: Vec<BedrockGuardrailOutput> => "outputs", assessments: Vec<BedrockGuardrailAssessment> => "assessments" });

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BedrockChecksTextContent {
    #[serde(default)]
    pub text: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BedrockChecksMessage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<Vec<BedrockChecksTextContent>>,
}
record!(BedrockChecksScoreEntry { category: String => "category", severity_score: f64 => "severityScore" });
record!(BedrockChecksPiiEntry { type_name: String => "type", confidence_score: f64 => "confidenceScore", message_index: i64 => "messageIndex", content_index: i64 => "contentIndex", begin_offset: i64 => "beginOffset", end_offset: i64 => "endOffset" });
record!(BedrockChecksScoreResult { results: Vec<BedrockChecksScoreEntry> => "results" });
record!(BedrockChecksSensitiveInformationResult { results: Vec<BedrockChecksPiiEntry> => "results", truncated: bool => "truncated" });
record!(BedrockChecksResults { content_filter: BedrockChecksScoreResult => "contentFilter", prompt_attack: BedrockChecksScoreResult => "promptAttack", sensitive_information: BedrockChecksSensitiveInformationResult => "sensitiveInformation" });
record!(BedrockChecksViolation { check: String => "check", category: String => "category", type_name: String => "type", severity_score: f64 => "severityScore", confidence_score: f64 => "confidenceScore", truncated: bool => "truncated" });
record!(BedrockChecksTextUnits { text_units: i64 => "textUnits" });
record!(BedrockChecksUsage { content_filter: BedrockChecksTextUnits => "contentFilter", prompt_attack: BedrockChecksTextUnits => "promptAttack", sensitive_information: BedrockChecksTextUnits => "sensitiveInformation" });
record!(BedrockGuardrailChecksResponse { results: BedrockChecksResults => "results", usage: BedrockChecksUsage => "usage" });
