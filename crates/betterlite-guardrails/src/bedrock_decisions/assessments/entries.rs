use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::BLOCKED;
use crate::bedrock_records::{
    Assessment, ContentFilter, GroundingFilter, ManagedWord, PiiEntity, Regex, TopicPolicyItem,
};

// a policy field keeps its type until entry renders it; an absent field renders as null
enum FieldValue<'field> {
    Text(Option<&'field String>),
    Number(Option<&'field Number>),
}

// every entry starts with its category, then the policy's own fields in LiteLLM's order
fn entry(category: &str, fields: &[(&str, FieldValue<'_>)]) -> Json {
    let mut object = IndexMap::from([("category".to_owned(), Json::Text(category.to_owned()))]);
    for (key, field) in fields {
        let value = match field {
            FieldValue::Text(text) => text.cloned().map_or(Json::Null, Json::Text),
            FieldValue::Number(number) => number.cloned().map_or(Json::Null, Json::Number),
        };
        object.insert((*key).to_owned(), value);
    }
    Json::Object(object)
}

fn blocked_entries<T, F>(items: Option<&[T]>, keep: F) -> Vec<Json>
where
    F: Fn(&T) -> Option<Json>,
{
    let mut kept = Vec::new();
    for item in items.unwrap_or_default() {
        kept.extend(keep(item));
    }
    kept
}

// a policy list plus the action getter that decides whether a match blocks, and the entry it builds
fn blocked_policy<T, A, E>(items: Option<&[T]>, action: A, build: E) -> Vec<Json>
where
    A: Fn(&T) -> Option<&str>,
    E: Fn(&T) -> Json,
{
    blocked_entries(items, |item| {
        (action(item) == Some(BLOCKED)).then(|| build(item))
    })
}

// LiteLLM's _extract_blocked_assessments appends topic, content, word, sensitive, then contextual grounding
pub(super) fn assessment_blocks(assessment: &Assessment) -> Vec<(&'static str, Vec<Json>)> {
    let topic = assessment
        .topic_policy
        .as_ref()
        .and_then(|policy| policy.topics.as_deref());
    let content = assessment
        .content_policy
        .as_ref()
        .and_then(|policy| policy.filters.as_deref());
    let grounding = assessment
        .contextual_grounding_policy
        .as_ref()
        .and_then(|policy| policy.filters.as_deref());
    let mut blocks = vec![
        (
            "topicPolicy",
            blocked_policy(topic, |item| item.action.as_deref(), topic_entry),
        ),
        (
            "contentPolicy",
            blocked_policy(content, |item| item.action.as_deref(), content_entry),
        ),
    ];
    blocks.extend(word_and_sensitive_blocks(assessment));
    blocks.push((
        "contextualGroundingPolicy",
        blocked_policy(grounding, |item| item.action.as_deref(), grounding_entry),
    ));
    blocks
}

fn word_and_sensitive_blocks(assessment: &Assessment) -> [(&'static str, Vec<Json>); 2] {
    let word_policy = assessment.word_policy.as_ref();
    let custom = word_policy.and_then(|policy| policy.custom_words.as_deref());
    let managed = word_policy.and_then(|policy| policy.managed_word_lists.as_deref());
    let sensitive = assessment.sensitive_information_policy.as_ref();
    let entities = sensitive.and_then(|policy| policy.pii_entities.as_deref());
    let patterns = sensitive.and_then(|policy| policy.regexes.as_deref());
    let mut words = blocked_entries(custom, |word| {
        (word.action.as_deref() == Some(BLOCKED)).then(|| {
            entry(
                "customWords",
                &[
                    ("match", FieldValue::Text(word.match_text.as_ref())),
                    ("action", FieldValue::Text(word.action.as_ref())),
                ],
            )
        })
    });
    words.extend(blocked_policy(
        managed,
        |word| word.action.as_deref(),
        managed_entry,
    ));
    [
        ("wordPolicy", words),
        (
            "sensitiveInformationPolicy",
            [
                blocked_policy(entities, |entity| entity.action.as_deref(), pii_entry),
                blocked_policy(patterns, |pattern| pattern.action.as_deref(), regex_entry),
            ]
            .concat(),
        ),
    ]
}

fn topic_entry(topic: &TopicPolicyItem) -> Json {
    entry(
        "topics",
        &[
            ("name", FieldValue::Text(topic.name.as_ref())),
            ("type", FieldValue::Text(topic.type_name.as_ref())),
            ("action", FieldValue::Text(topic.action.as_ref())),
        ],
    )
}

// content filters and contextual grounding filters share the category and the outer fields
fn filter_entry(
    type_name: Option<&String>,
    middle: [(&'static str, FieldValue<'_>); 2],
    action: Option<&String>,
) -> Json {
    let [first, second] = middle;
    entry(
        "filters",
        &[
            ("type", FieldValue::Text(type_name)),
            first,
            second,
            ("action", FieldValue::Text(action)),
        ],
    )
}

const fn content_middle(filter: &ContentFilter) -> [(&'static str, FieldValue<'_>); 2] {
    [
        ("confidence", FieldValue::Text(filter.confidence.as_ref())),
        (
            "filterStrength",
            FieldValue::Text(filter.filter_strength.as_ref()),
        ),
    ]
}

const fn grounding_middle(filter: &GroundingFilter) -> [(&'static str, FieldValue<'_>); 2] {
    [
        ("threshold", FieldValue::Number(filter.threshold.as_ref())),
        ("score", FieldValue::Number(filter.score.as_ref())),
    ]
}

fn content_entry(filter: &ContentFilter) -> Json {
    filter_entry(
        filter.type_name.as_ref(),
        content_middle(filter),
        filter.action.as_ref(),
    )
}

fn grounding_entry(filter: &GroundingFilter) -> Json {
    filter_entry(
        filter.type_name.as_ref(),
        grounding_middle(filter),
        filter.action.as_ref(),
    )
}

// managed word lists and PII entities carry the same type, match and action fields
fn typed_match_entry(
    category: &str,
    type_name: Option<&String>,
    match_text: Option<&String>,
    action: Option<&String>,
) -> Json {
    entry(
        category,
        &[
            ("type", FieldValue::Text(type_name)),
            ("match", FieldValue::Text(match_text)),
            ("action", FieldValue::Text(action)),
        ],
    )
}

fn managed_entry(word: &ManagedWord) -> Json {
    typed_match_entry(
        "managedWordLists",
        word.type_name.as_ref(),
        word.match_text.as_ref(),
        word.action.as_ref(),
    )
}

fn pii_entry(entity: &PiiEntity) -> Json {
    typed_match_entry(
        "piiEntities",
        entity.type_name.as_ref(),
        entity.match_text.as_ref(),
        entity.action.as_ref(),
    )
}

fn regex_entry(pattern: &Regex) -> Json {
    entry(
        "regexes",
        &[
            ("name", FieldValue::Text(pattern.name.as_ref())),
            ("regex", FieldValue::Text(pattern.regex.as_ref())),
            ("match", FieldValue::Text(pattern.match_text.as_ref())),
            ("action", FieldValue::Text(pattern.action.as_ref())),
        ],
    )
}
