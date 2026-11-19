use betterlite_db::compliance_redaction::{RedactionKeys, redact_nested_match_and_regex_keys};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::bedrock_records::{Assessment, Response};

pub const BLOCKED: &str = "BLOCKED";
pub const INTERVENED: &str = "GUARDRAIL_INTERVENED";
pub const VIOLATED_POLICY: &str = "Violated guardrail policy";

// LiteLLM raises for a BLOCKED action only; ANONYMIZED and NONE keep the content path going
#[must_use]
pub fn should_raise_blocked(response: &Response) -> bool {
    response.action.as_deref() == Some(INTERVENED) && !blocked_assessments(response).is_empty()
}

// LiteLLM's _extract_blocked_assessments: one {policy, matches} entry per policy with a BLOCKED match
#[must_use]
pub fn blocked_assessments(response: &Response) -> Vec<Json> {
    let mut blocked = Vec::new();
    for assessment in response.assessments.as_deref().unwrap_or_default() {
        push_block(&mut blocked, "topicPolicy", topic_matches(assessment));
        push_block(&mut blocked, "contentPolicy", content_matches(assessment));
        push_block(&mut blocked, "wordPolicy", word_matches(assessment));
        push_block(
            &mut blocked,
            "sensitiveInformationPolicy",
            sensitive_matches(assessment),
        );
        push_block(
            &mut blocked,
            "contextualGroundingPolicy",
            grounding_matches(assessment),
        );
    }
    blocked
}

// only the non-sensitive labels reach telemetry, never the raw match text
#[must_use]
pub fn violation_category_names(response: &Response) -> Vec<String> {
    let mut names = Vec::new();
    for block in blocked_assessments(response) {
        let Json::Object(fields) = block else {
            continue;
        };
        let Some(Json::List(matches)) = fields.get("matches") else {
            continue;
        };
        for entry in matches {
            let Json::Object(candidate) = entry else {
                continue;
            };
            if let Some(label) = category_label(candidate) {
                names.push(label.to_owned());
            }
        }
    }
    names
}

#[must_use]
pub fn redact_assessments(assessments: &[Json]) -> Vec<Json> {
    let keys = RedactionKeys::default();
    // ast-grep-ignore: no-long-names
    let redacted = redact_nested_match_and_regex_keys(&Json::List(assessments.to_vec()), &keys);
    if let Json::List(items) = redacted {
        return items;
    }
    assessments.to_vec()
}

fn category_label(fields: &IndexMap<String, Json>) -> Option<&str> {
    label_text(fields.get("name")).or(label_text(fields.get("type")))
}

fn label_text(value: Option<&Json>) -> Option<&str> {
    let Some(Json::Text(text)) = value else {
        return None;
    };
    (!text.is_empty()).then_some(text.as_str())
}

fn push_block(blocked: &mut Vec<Json>, policy: &str, matches: Vec<Json>) {
    if matches.is_empty() {
        return;
    }
    blocked.push(Json::Object(IndexMap::from([
        ("policy".to_owned(), Json::Text(policy.to_owned())),
        ("matches".to_owned(), Json::List(matches)),
    ])));
}

fn topic_matches(assessment: &Assessment) -> Vec<Json> {
    let Some(policy) = assessment.topic_policy.as_ref() else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for topic in policy.topics.as_deref().unwrap_or_default() {
        if topic.action.as_deref() == Some(BLOCKED) {
            matches.push(Json::Object(IndexMap::from([
                ("category".to_owned(), Json::Text("topics".to_owned())),
                ("name".to_owned(), optional_text(topic.name.as_ref())),
                ("type".to_owned(), optional_text(topic.type_name.as_ref())),
                ("action".to_owned(), optional_text(topic.action.as_ref())),
            ])));
        }
    }
    matches
}

fn content_matches(assessment: &Assessment) -> Vec<Json> {
    let Some(policy) = assessment.content_policy.as_ref() else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for filter in policy.filters.as_deref().unwrap_or_default() {
        if filter.action.as_deref() == Some(BLOCKED) {
            matches.push(Json::Object(IndexMap::from([
                ("category".to_owned(), Json::Text("filters".to_owned())),
                ("type".to_owned(), optional_text(filter.type_name.as_ref())),
                (
                    "confidence".to_owned(),
                    optional_text(filter.confidence.as_ref()),
                ),
                (
                    "filterStrength".to_owned(),
                    optional_text(filter.filter_strength.as_ref()),
                ),
                ("action".to_owned(), optional_text(filter.action.as_ref())),
            ])));
        }
    }
    matches
}

fn word_matches(assessment: &Assessment) -> Vec<Json> {
    let Some(policy) = assessment.word_policy.as_ref() else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for custom in policy.custom_words.as_deref().unwrap_or_default() {
        if custom.action.as_deref() == Some(BLOCKED) {
            matches.push(Json::Object(IndexMap::from([
                ("category".to_owned(), Json::Text("customWords".to_owned())),
                (
                    "match".to_owned(),
                    optional_text(custom.match_text.as_ref()),
                ),
                ("action".to_owned(), optional_text(custom.action.as_ref())),
            ])));
        }
    }
    for managed in policy.managed_word_lists.as_deref().unwrap_or_default() {
        if managed.action.as_deref() == Some(BLOCKED) {
            matches.push(Json::Object(IndexMap::from([
                (
                    "category".to_owned(),
                    Json::Text("managedWordLists".to_owned()),
                ),
                ("type".to_owned(), optional_text(managed.type_name.as_ref())),
                (
                    "match".to_owned(),
                    optional_text(managed.match_text.as_ref()),
                ),
                ("action".to_owned(), optional_text(managed.action.as_ref())),
            ])));
        }
    }
    matches
}

fn sensitive_matches(assessment: &Assessment) -> Vec<Json> {
    let Some(policy) = assessment.sensitive_information_policy.as_ref() else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for entity in policy.pii_entities.as_deref().unwrap_or_default() {
        if entity.action.as_deref() == Some(BLOCKED) {
            matches.push(Json::Object(IndexMap::from([
                ("category".to_owned(), Json::Text("piiEntities".to_owned())),
                ("type".to_owned(), optional_text(entity.type_name.as_ref())),
                (
                    "match".to_owned(),
                    optional_text(entity.match_text.as_ref()),
                ),
                ("action".to_owned(), optional_text(entity.action.as_ref())),
            ])));
        }
    }
    for pattern in policy.regexes.as_deref().unwrap_or_default() {
        if pattern.action.as_deref() == Some(BLOCKED) {
            matches.push(Json::Object(IndexMap::from([
                ("category".to_owned(), Json::Text("regexes".to_owned())),
                ("name".to_owned(), optional_text(pattern.name.as_ref())),
                ("regex".to_owned(), optional_text(pattern.regex.as_ref())),
                (
                    "match".to_owned(),
                    optional_text(pattern.match_text.as_ref()),
                ),
                ("action".to_owned(), optional_text(pattern.action.as_ref())),
            ])));
        }
    }
    matches
}

fn grounding_matches(assessment: &Assessment) -> Vec<Json> {
    let Some(policy) = assessment.contextual_grounding_policy.as_ref() else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for filter in policy.filters.as_deref().unwrap_or_default() {
        if filter.action.as_deref() == Some(BLOCKED) {
            matches.push(Json::Object(IndexMap::from([
                ("category".to_owned(), Json::Text("filters".to_owned())),
                ("type".to_owned(), optional_text(filter.type_name.as_ref())),
                (
                    "threshold".to_owned(),
                    optional_number(filter.threshold.as_ref()),
                ),
                ("score".to_owned(), optional_number(filter.score.as_ref())),
                ("action".to_owned(), optional_text(filter.action.as_ref())),
            ])));
        }
    }
    matches
}

fn optional_text(value: Option<&String>) -> Json {
    value.map_or(Json::Null, |text| Json::Text(text.clone()))
}

fn optional_number(value: Option<&Number>) -> Json {
    value.map_or(Json::Null, |number| Json::Number(number.clone()))
}
