use super::{choice_message, joined_choice, optional_text, text_choice};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::payloads::helpers::{dicts, float_value};
use crate::otel::model::utils::coercion::{OtelModelValue, as_int};

pub(super) fn completion_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    let Some(choices) = response.get("choices") else {
        return Vec::new();
    };
    dicts(choices).into_iter().map(completion_choice).collect()
}

fn completion_choice(choice: MetadataMap) -> MetadataMap {
    let text = choice
        .get("text")
        .and_then(OtelModelValue::as_str)
        .map(str::to_owned);
    match (choice.contains_key("message"), text) {
        (false, Some(content)) => text_completion_choice(&choice, &content),
        _ => choice,
    }
}

fn text_completion_choice(choice: &MetadataMap, text: &str) -> MetadataMap {
    let mut rewritten = choice.clone();
    rewritten.remove("text");
    let message = choice_message("assistant", Some(text), None, None);
    rewritten.insert("message".to_owned(), OtelModelValue::Object(message));
    let finish_reason = choice.get("finish_reason").and_then(OtelModelValue::as_str);
    rewritten.insert("finish_reason".to_owned(), optional_text(finish_reason));
    rewritten
}

pub(super) fn ocr_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    let Some(pages) = response.get("pages") else {
        return Vec::new();
    };
    joined_choice(dicts(pages).iter().filter_map(page_markdown))
}

fn page_markdown(page: &MetadataMap) -> Option<String> {
    page.get("markdown")
        .and_then(OtelModelValue::as_str)
        .map(str::to_owned)
}

pub(super) fn transcription_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    match response.get("text").and_then(OtelModelValue::as_str) {
        Some(text) if !text.is_empty() => vec![text_choice(Some(text), None)],
        Some(_) | None => Vec::new(),
    }
}

pub(super) fn moderation_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    let Some(results) = response.get("results") else {
        return Vec::new();
    };
    joined_choice(dicts(results).iter().filter_map(verdict_line))
}

fn verdict_line(result: &MetadataMap) -> Option<String> {
    let flagged = result.get("flagged").and_then(OtelModelValue::as_bool)?;
    if flagged {
        return Some(flagged_hits(result));
    }
    Some("not flagged".to_owned())
}

fn flagged_hits(result: &MetadataMap) -> String {
    let Some(categories) = result.get("categories").and_then(OtelModelValue::as_object) else {
        return "flagged".to_owned();
    };
    let hits: Vec<&str> = categories
        .iter()
        .filter(|(_, hit)| matches!(hit, OtelModelValue::Bool(true)))
        .map(|(name, _)| name.as_str())
        .collect();
    if hits.is_empty() {
        return "flagged".to_owned();
    }
    format!("flagged: {}", hits.join(", "))
}

pub(super) fn rerank_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    let Some(results) = response.get("results") else {
        return Vec::new();
    };
    joined_choice(dicts(results).iter().filter_map(rerank_line))
}

fn rerank_line(result: &MetadataMap) -> Option<String> {
    let index = result.get("index").and_then(as_int)?;
    let score = result.get("relevance_score").and_then(float_value)?;
    Some(rerank_text(index, score, document_text(result).as_deref()))
}

fn rerank_text(index: i64, score: f64, text: Option<&str>) -> String {
    text.map_or_else(
        || format!("[{index}] {score:?}"),
        |value| format!("[{index}] {score:?}\n{value}"),
    )
}

fn document_text(result: &MetadataMap) -> Option<String> {
    result
        .get("document")
        .and_then(OtelModelValue::as_object)
        .and_then(|document| document.get("text"))
        .and_then(OtelModelValue::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

pub(super) fn search_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    let Some(results) = response.get("results") else {
        return Vec::new();
    };
    joined_choice(dicts(results).iter().filter_map(search_line))
}

fn search_line(result: &MetadataMap) -> Option<String> {
    let mut parts = Vec::new();
    for key in ["title", "url", "snippet"] {
        let part = result.get(key).and_then(OtelModelValue::as_str);
        if let Some(value) = part.filter(|value| !value.is_empty()) {
            parts.push(value);
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

pub(super) fn image_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    let Some(data) = response.get("data") else {
        return Vec::new();
    };
    joined_choice(dicts(data).iter().filter_map(image_summary))
}

fn image_summary(item: &MetadataMap) -> Option<String> {
    let location = image_location(item)?;
    Some(match revised_prompt(item) {
        Some(prompt) => format!("{prompt}\n{location}"),
        None => location,
    })
}

fn revised_prompt(item: &MetadataMap) -> Option<&str> {
    item.get("revised_prompt")
        .and_then(OtelModelValue::as_str)
        .filter(|prompt| !prompt.is_empty())
}

fn image_location(item: &MetadataMap) -> Option<String> {
    if let Some(url) = item.get("url").and_then(OtelModelValue::as_str) {
        return Some(url.to_owned());
    }
    let encoded = item.get("b64_json").and_then(OtelModelValue::as_str)?;
    let length = i64::try_from(encoded.chars().count()).ok()?;
    let padding = i64::try_from(
        encoded
            .chars()
            .rev()
            .take(2)
            .filter(|character| *character == '=')
            .count(),
    )
    .ok()?;
    let scaled = length.checked_mul(3)?.checked_div(4)?;
    let size = scaled.checked_sub(padding)?;
    Some(format!("b64_json image ({size} bytes)"))
}

pub(super) fn binary_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    let Some(size) = response.get("num_bytes").and_then(as_int) else {
        return Vec::new();
    };
    vec![text_choice(Some(&binary_summary(response, size)), None)]
}

fn binary_summary(response: &MetadataMap, size: i64) -> String {
    let content_type = response
        .get("content_type")
        .and_then(OtelModelValue::as_str);
    let label = content_type.filter(|value| !value.is_empty());
    label.map_or_else(
        || format!("{size} bytes"),
        |value| format!("{value} ({size} bytes)"),
    )
}
