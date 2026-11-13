use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

#[path = "output_choices/responses.rs"]
mod responses;
#[path = "output_choices/summaries.rs"]
mod summaries;

#[must_use]
pub fn output_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    let readers: [fn(&MetadataMap) -> Vec<MetadataMap>; 9] = [
        summaries::completion_choices,
        responses::responses_choices,
        summaries::ocr_choices,
        summaries::transcription_choices,
        summaries::moderation_choices,
        summaries::rerank_choices,
        summaries::search_choices,
        summaries::image_choices,
        summaries::binary_choices,
    ];
    readers
        .into_iter()
        .map(|reader| reader(response))
        .find(|choices| !choices.is_empty())
        .unwrap_or_default()
}

#[must_use]
pub fn finish_reasons(choices: &[MetadataMap]) -> Vec<String> {
    let mut reasons = Vec::new();
    for choice in choices {
        let reason = choice
            .get("finish_reason")
            .and_then(OtelModelValue::as_str)
            .filter(|reason| !reason.is_empty());
        if let Some(value) = reason {
            reasons.push(value.to_owned());
        }
    }
    reasons
}

pub(super) fn text_choice(content: &str, finish_reason: Option<&str>) -> MetadataMap {
    let message = choice_message("assistant", Some(content), None, None);
    BTreeMap::from([
        ("finish_reason".to_owned(), optional_text(finish_reason)),
        ("message".to_owned(), OtelModelValue::Object(message)),
    ])
}

pub(super) fn joined_choice<Parts>(items: Parts) -> Vec<MetadataMap>
where
    Parts: IntoIterator<Item = String>,
{
    let parts: Vec<String> = items.into_iter().collect();
    if parts.is_empty() {
        return Vec::new();
    }
    vec![text_choice(&parts.join("\n\n"), None)]
}

pub(super) fn choice_message(
    role: &str,
    content: Option<&str>,
    refusal: Option<&str>,
    tool_calls: Option<Vec<MetadataMap>>,
) -> MetadataMap {
    let calls_value = tool_calls.map_or(OtelModelValue::Null(()), |calls| {
        OtelModelValue::Array(calls.into_iter().map(OtelModelValue::Object).collect())
    });
    BTreeMap::from([
        ("content".to_owned(), optional_text(content)),
        ("refusal".to_owned(), optional_text(refusal)),
        ("role".to_owned(), OtelModelValue::String(role.to_owned())),
        ("tool_calls".to_owned(), calls_value),
    ])
}

pub(super) fn optional_text(text: Option<&str>) -> OtelModelValue {
    text.map_or_else(
        || OtelModelValue::Null(()),
        |value| OtelModelValue::String(value.to_owned()),
    )
}

#[path = "output_choices/tests.rs"]
mod tests;
