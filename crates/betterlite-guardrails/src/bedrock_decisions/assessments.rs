use betterlite_db::compliance_redaction::{self, RedactionKeys};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use self::entries::assessment_blocks;
use crate::bedrock_records::Response;

mod entries;

pub const BLOCKED: &str = "BLOCKED";
pub const INTERVENED: &str = "GUARDRAIL_INTERVENED";
pub const VIOLATED_POLICY: &str = "Violated guardrail policy";

// LiteLLM raises for a BLOCKED action only; ANONYMIZED and NONE keep the content path going
#[must_use]
pub fn should_raise_blocked(response: &Response) -> bool {
    if response.action.as_deref() != Some(INTERVENED) {
        return false;
    }
    !blocked_assessments(response).is_empty()
}

// LiteLLM's _extract_blocked_assessments: one {policy, matches} entry per policy with a BLOCKED match
#[must_use]
pub fn blocked_assessments(response: &Response) -> Vec<Json> {
    let mut blocked = Vec::new();
    for assessment in response.assessments.as_deref().unwrap_or_default() {
        for (policy, matches) in assessment_blocks(assessment) {
            push_block(&mut blocked, policy, matches);
        }
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
            let name = label_text(candidate.get("name"));
            let kind = label_text(candidate.get("type"));
            if let Some(label) = name.or(kind) {
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
    let redacted = compliance_redaction::redact_nested_match_and_regex_keys(
        &Json::List(assessments.to_vec()),
        &keys,
    );
    if let Json::List(items) = redacted {
        return items;
    }
    assessments.to_vec()
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

fn label_text(value: Option<&Json>) -> Option<&str> {
    let Some(Json::Text(text)) = value else {
        return None;
    };
    (!text.is_empty()).then_some(text.as_str())
}
