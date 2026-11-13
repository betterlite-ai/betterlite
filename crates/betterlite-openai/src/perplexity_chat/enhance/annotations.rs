use std::collections::BTreeMap;

use fancy_regex::Regex;
use serde_json::{Map, Value};

use super::PerplexityReplyError;
use crate::perplexity_chat::json_truthy;

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
    let empty = AnnotationPlan {
        annotations: Vec::new(),
        citations: None,
        search_results: None,
    };
    let choices = match reply.get("choices") {
        None | Some(Value::Null) => return Ok(empty),
        Some(Value::Array(choices)) => choices.as_slice(),
        Some(other) if !json_truthy(other) => return Ok(empty),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_)) => {
            return Err(PerplexityReplyError::Shape { field: "choices" });
        }
    };
    let Some(choice) = choices.first() else {
        return Ok(empty);
    };
    let Value::Object(choice) = choice else {
        return Ok(empty);
    };
    let content = match choice.get("message") {
        None | Some(Value::Null) => return Ok(empty),
        Some(Value::Object(message)) => match message.get("content") {
            None | Some(Value::Null) => return Ok(empty),
            Some(Value::String(text)) => text.as_str(),
            Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
                return Err(PerplexityReplyError::Shape { field: "content" });
            }
        },
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            return Ok(empty);
        }
    };
    if content.is_empty() {
        return Ok(empty);
    }
    let citations = match raw.get("citations") {
        None => &[][..],
        Some(Value::Array(citations)) => citations.as_slice(),
        Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
        | Some(Value::Object(_)) => {
            return Err(PerplexityReplyError::Shape { field: "citations" });
        }
    };
    let search_results = match raw.get("search_results") {
        None => &[][..],
        Some(Value::Array(results)) => results.as_slice(),
        Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
        | Some(Value::Object(_)) => {
            return Err(PerplexityReplyError::Shape {
                field: "search_results",
            });
        }
    };
    let mut titles: BTreeMap<&str, &str> = BTreeMap::new();
    for result in search_results {
        if let Value::Object(fields) = result
            && let (Some(Value::String(url)), Some(Value::String(title))) =
                (fields.get("url"), fields.get("title"))
        {
            titles.insert(url.as_str(), title.as_str());
        }
    }
    let mut urls: BTreeMap<usize, &str> = BTreeMap::new();
    for (index, citation) in citations.iter().enumerate() {
        if let Value::String(url) = citation
            && let Some(number) = index.checked_add(1)
        {
            urls.insert(number, url.as_str());
        }
    }
    let annotations = markers(content, &urls, &titles)?;
    if !annotations.is_empty()
        && let Some(existing) = existing_annotations(reply)
        && !matches!(existing, Value::Array(_) | Value::Null)
    {
        // the pinned code calls .extend on whatever the message's annotations hold
        return Err(PerplexityReplyError::Shape {
            field: "annotations",
        });
    }
    let citations_copy = (!citations.is_empty()).then(|| Value::Array(citations.to_vec()));
    let search_results_copy =
        (!search_results.is_empty()).then(|| Value::Array(search_results.to_vec()));
    Ok(AnnotationPlan {
        annotations,
        citations: citations_copy,
        search_results: search_results_copy,
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
    for found in pattern.find_iter(content) {
        let found = found.map_err(PerplexityReplyError::Marker)?;
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
