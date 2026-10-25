use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Number;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

mod tests;

pub type Extensions = BTreeMap<String, Box<RawValue>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Qualifier {
    GroundingSource,
    Query,
    GuardContent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Source {
    Input,
    Output,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

// An explicit null and any key AWS adds later stay in `extra`, so a record serializes back to what was read.
macro_rules! record { ($name:ident { $($field:ident: $ty:ty => $wire:literal),* $(,)? }) => {
    #[derive(Debug, Clone, Default, Serialize)]
    pub struct $name { $(#[serde(rename = $wire, skip_serializing_if = "Option::is_none")] pub $field: Option<$ty>,)* #[serde(flatten)] pub extra: Extensions }
    impl $name {
        pub fn absorb(&mut self, other: &Self) {
            $(if other.$field.is_some() { self.$field.clone_from(&other.$field); self.extra.remove($wire); })*
            for (key, raw) in &other.extra {
                $(if key == $wire { self.$field = None; })*
                self.extra.insert(key.clone(), raw.clone());
            }
        }
    }
    impl<'de> Deserialize<'de> for $name {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: Deserializer<'de> {
            let mut record = Self::default();
            for (key, raw) in Extensions::deserialize(deserializer)? {
                match key.as_str() {
                    $($wire if raw.get() != "null" => record.$field = Some(serde_json::from_str(raw.get()).map_err(D::Error::custom)?),)*
                    _ => { record.extra.insert(key, raw); }
                }
            }
            Ok(record)
        }
    }
}; }
record!(TextContent { text: String => "text", qualifiers: Vec<Qualifier> => "qualifiers" });
record!(ContentItem { text: TextContent => "text" });
record!(Request { source: Source => "source", content: Vec<ContentItem> => "content" });
record!(Output { text: String => "text" });
record!(ChecksTextContent { text: String => "text" });
record!(ChecksMessage { role: Role => "role", content: Vec<ChecksTextContent> => "content" });
record!(TopicPolicyItem { name: String => "name", type_name: String => "type", action: String => "action" });
record!(TopicPolicy { topics: Vec<TopicPolicyItem> => "topics" });
record!(ContentFilter { type_name: String => "type", confidence: String => "confidence", filter_strength: String => "filterStrength", action: String => "action" });
record!(ContentPolicy { filters: Vec<ContentFilter> => "filters" });
record!(CustomWord { match_text: String => "match", action: String => "action" });
record!(ManagedWord { match_text: String => "match", type_name: String => "type", action: String => "action" });
record!(WordPolicy { custom_words: Vec<CustomWord> => "customWords", managed_word_lists: Vec<ManagedWord> => "managedWordLists" });
record!(PiiEntity { type_name: String => "type", match_text: String => "match", action: String => "action" });
record!(Regex { name: String => "name", regex: String => "regex", match_text: String => "match", action: String => "action" });
record!(SensitivePolicy { pii_entities: Vec<PiiEntity> => "piiEntities", regexes: Vec<Regex> => "regexes" });
record!(GroundingFilter { type_name: String => "type", threshold: Number => "threshold", score: Number => "score", action: String => "action" });
record!(GroundingPolicy { filters: Vec<GroundingFilter> => "filters" });
record!(Coverage { text_characters: BTreeMap<String, i64> => "textCharacters" });
record!(InvocationMetrics { guardrail_processing_latency: i64 => "guardrailProcessingLatency", usage: Usage => "usage", guardrail_coverage: Coverage => "guardrailCoverage" });
record!(Assessment { topic_policy: TopicPolicy => "topicPolicy", content_policy: ContentPolicy => "contentPolicy", word_policy: WordPolicy => "wordPolicy", sensitive_information_policy: SensitivePolicy => "sensitiveInformationPolicy", contextual_grounding_policy: GroundingPolicy => "contextualGroundingPolicy", invocation_metrics: InvocationMetrics => "invocationMetrics", guardrail_coverage: Coverage => "guardrailCoverage" });
record!(Response { usage: Usage => "usage", action: String => "action", output: Vec<Output> => "output", outputs: Vec<Output> => "outputs", assessments: Vec<Assessment> => "assessments" });

record!(ChecksScoreEntry { category: String => "category", severity_score: Number => "severityScore" });
record!(ChecksPiiEntry { type_name: String => "type", confidence_score: Number => "confidenceScore", message_index: i64 => "messageIndex", content_index: i64 => "contentIndex", begin_offset: i64 => "beginOffset", end_offset: i64 => "endOffset" });
record!(ChecksScoreResult { results: Vec<ChecksScoreEntry> => "results" });
record!(ChecksSensitiveResult { results: Vec<ChecksPiiEntry> => "results", truncated: bool => "truncated" });
record!(ChecksResults { content_filter: ChecksScoreResult => "contentFilter", prompt_attack: ChecksScoreResult => "promptAttack", sensitive_information: ChecksSensitiveResult => "sensitiveInformation" });
record!(ChecksViolation { check: String => "check", category: String => "category", type_name: String => "type", severity_score: Number => "severityScore", confidence_score: Number => "confidenceScore", truncated: bool => "truncated" });
record!(ChecksTextUnits { text_units: i64 => "textUnits" });
record!(ChecksUsage { content_filter: ChecksTextUnits => "contentFilter", prompt_attack: ChecksTextUnits => "promptAttack", sensitive_information: ChecksTextUnits => "sensitiveInformation" });
record!(ChecksResponse { results: ChecksResults => "results", usage: ChecksUsage => "usage" });

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Usage(pub BTreeMap<String, Option<i64>>);
#[derive(Debug, Clone)]
pub struct QualifiedTextBlock {
    pub text: String,
    pub qualifier: Option<Qualifier>,
}
#[derive(Debug, Clone)]
pub struct MessageFilterResult<Message> {
    pub payload_messages: Option<Vec<Message>>,
    pub original_messages: Option<Vec<Message>>,
    pub target_indices: Option<Vec<usize>>,
}
#[derive(Debug, Clone)]
pub struct MessageSelection<Message> {
    pub filtered_messages: Option<Vec<Message>>,
    pub scanned_slice: Option<(usize, usize)>,
    pub scanned_role_subset: bool,
    pub skip_scan: bool,
}
