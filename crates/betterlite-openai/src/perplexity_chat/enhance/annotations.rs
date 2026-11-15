use std::collections::BTreeMap;

use fancy_regex::Regex;
use serde_json::{Map, Value};

use super::PerplexityReplyError;

const MARKER_PATTERN: &str = r"\[([0-9]+)\]";

pub(super) struct AnnotationPlan {
    annotations: Vec<Value>,
    citations: Option<Value>,
    search_results: Option<Value>,
}

impl AnnotationPlan {
    pub(super) fn apply(self, reply: &mut Map<String, Value>) {
        if !self.annotations.is_empty() {
            append_annotations(reply, &self.annotations);
        }
        if let Some(citations) = self.citations {
            reply.insert(String::from("citations"), citations);
        }
        if let Some(search_results) = self.search_results {
            reply.insert(String::from("search_results"), search_results);
        }
    }
}

fn append_annotations(reply: &mut Map<String, Value>, annotations: &[Value]) {
    let Some(Value::Array(choices)) = reply.get_mut("choices") else {
        return;
    };
    let Some(Value::Object(choice)) = choices.first_mut() else {
        return;
    };
    let Some(Value::Object(message)) = choice.get_mut("message") else {
        return;
    };
    if !matches!(message.get("annotations"), Some(Value::Array(_))) {
        message.insert(String::from("annotations"), Value::Array(Vec::new()));
    }
    if let Some(Value::Array(existing)) = message.get_mut("annotations") {
        existing.extend(annotations.iter().cloned());
    }
}

pub(super) fn plan(
    reply: &Map<String, Value>,
    raw: &Map<String, Value>,
) -> Result<AnnotationPlan, PerplexityReplyError> {
    let Some(content) = message_content(reply)? else {
        return Ok(AnnotationPlan {
            annotations: Vec::new(),
            citations: None,
            search_results: None,
        });
    };
    let citations = match raw.get("citations") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(citations)) => citations.as_slice(),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_)) => {
            return Err(PerplexityReplyError::Shape { field: "citations" });
        }
    };
    let search_results = match raw.get("search_results") {
        None => &[][..],
        Some(Value::Array(results)) => results.as_slice(),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_),
        ) => {
            return Err(PerplexityReplyError::Shape {
                field: "search_results",
            });
        }
    };
    let annotations = markers(content, &urls(citations), &titles(search_results))?;
    check_existing_annotations(reply, &annotations)?;
    Ok(AnnotationPlan {
        annotations,
        citations: (!citations.is_empty()).then(|| Value::Array(citations.to_vec())),
        search_results: (!search_results.is_empty()).then(|| Value::Array(search_results.to_vec())),
    })
}

fn message_content(reply: &Map<String, Value>) -> Result<Option<&str>, PerplexityReplyError> {
    let Some(choice) = first_choice(reply)? else {
        return Ok(None);
    };
    let Some(message) = choice.get("message").and_then(Value::as_object) else {
        return Ok(None);
    };
    match message.get("content") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok((!text.is_empty()).then_some(text.as_str())),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            Err(PerplexityReplyError::Shape { field: "content" })
        }
    }
}

fn first_choice(
    reply: &Map<String, Value>,
) -> Result<Option<&Map<String, Value>>, PerplexityReplyError> {
    match reply.get("choices") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(choices)) => Ok(choices.first().and_then(Value::as_object)),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_)) => {
            Err(PerplexityReplyError::Shape { field: "choices" })
        }
    }
}

fn titles(search_results: &[Value]) -> BTreeMap<&str, &str> {
    let mut titles = BTreeMap::new();
    for result in search_results {
        if let Value::Object(fields) = result
            && let (Some(Value::String(url)), Some(Value::String(title))) =
                (fields.get("url"), fields.get("title"))
        {
            titles.insert(url.as_str(), title.as_str());
        }
    }
    titles
}

fn urls(citations: &[Value]) -> BTreeMap<usize, &str> {
    let mut urls = BTreeMap::new();
    for (index, citation) in citations.iter().enumerate() {
        if let Value::String(url) = citation
            && let Some(number) = index.checked_add(1)
        {
            urls.insert(number, url.as_str());
        }
    }
    urls
}

fn check_existing_annotations(
    reply: &Map<String, Value>,
    annotations: &[Value],
) -> Result<(), PerplexityReplyError> {
    if annotations.is_empty() {
        return Ok(());
    }
    let Some(existing) = existing_annotations(reply) else {
        return Ok(());
    };
    if matches!(existing, Value::Array(_) | Value::Null) {
        return Ok(());
    }
    // the pinned code calls .extend on whatever the message's annotations hold
    Err(PerplexityReplyError::Shape {
        field: "annotations",
    })
}

fn existing_annotations(reply: &Map<String, Value>) -> Option<&Value> {
    let choices = reply.get("choices")?.as_array()?;
    let choice = choices.first()?.as_object()?;
    let message = choice.get("message")?.as_object()?;
    message.get("annotations")
}

fn markers(
    content: &str,
    urls: &BTreeMap<usize, &str>,
    titles: &BTreeMap<&str, &str>,
) -> Result<Vec<Value>, PerplexityReplyError> {
    let pattern = Regex::new(MARKER_PATTERN).map_err(PerplexityReplyError::MarkerPattern)?;
    let mut annotations = Vec::new();
    for candidate in pattern.find_iter(content) {
        let found = candidate.map_err(PerplexityReplyError::Marker)?;
        let text = found.as_str();
        let digits = text
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .ok_or(PerplexityReplyError::Shape { field: "content" })?;
        let Ok(number) = digits.parse::<usize>() else {
            continue;
        };
        let Some(url) = urls.get(&number) else {
            continue;
        };
        let title = titles.get(*url).copied().unwrap_or("");
        let start = content
            .get(..found.start())
            .ok_or(PerplexityReplyError::Shape { field: "content" })?
            .chars()
            .count();
        let end = content
            .get(..found.end())
            .ok_or(PerplexityReplyError::Shape { field: "content" })?
            .chars()
            .count();
        annotations.push(annotation(url, title, start, end));
    }
    Ok(annotations)
}

fn annotation(url: &str, title: &str, start: usize, end: usize) -> Value {
    let mut citation = Map::new();
    citation.insert(String::from("url"), Value::String(url.to_owned()));
    citation.insert(String::from("title"), Value::String(title.to_owned()));
    citation.insert(String::from("start_index"), Value::from(start));
    citation.insert(String::from("end_index"), Value::from(end));
    let mut entry = Map::new();
    entry.insert(
        String::from("type"),
        Value::String(String::from("url_citation")),
    );
    entry.insert(String::from("url_citation"), Value::Object(citation));
    Value::Object(entry)
}
