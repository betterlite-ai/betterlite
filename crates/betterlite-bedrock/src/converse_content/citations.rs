use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CitationBlocks {
    pub text: Option<String>,
    pub annotations: Option<Vec<CitationAnnotation>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CitationAnnotation {
    #[serde(rename = "type")]
    pub kind: String,
    pub url_citation: UrlCitation,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UrlCitation {
    pub start_index: Number,
    pub end_index: Number,
    pub title: String,
    pub url: String,
}

#[must_use]
pub fn citations_to_annotations(blocks: Option<&[Json]>) -> CitationBlocks {
    let Some(citations_blocks) = blocks.filter(|list| !list.is_empty()) else {
        return CitationBlocks {
            text: None,
            annotations: None,
        };
    };
    let mut text_parts: Vec<String> = Vec::new();
    let mut annotations = Vec::new();
    let mut offset: i128 = 0;
    for block in citations_blocks {
        collect_block(block, &mut offset, &mut text_parts, &mut annotations);
    }
    CitationBlocks {
        text: (!text_parts.is_empty()).then(|| text_parts.concat()),
        annotations: (!annotations.is_empty()).then_some(annotations),
    }
}

fn collect_block(
    block: &Json,
    offset: &mut i128,
    text_parts: &mut Vec<String>,
    annotations: &mut Vec<CitationAnnotation>,
) {
    let Json::Object(fields) = block else {
        return;
    };
    let block_text = citation_block_text(fields);
    let block_offset = *offset;
    if !block_text.is_empty() {
        let Ok(length) = i128::try_from(block_text.chars().count()) else {
            return;
        };
        let Some(next_offset) = block_offset.checked_add(length) else {
            return;
        };
        *offset = next_offset;
        text_parts.push(block_text);
    }
    collect_citations(fields, block_offset, annotations);
}

fn collect_citations(
    fields: &IndexMap<String, Json>,
    block_offset: i128,
    annotations: &mut Vec<CitationAnnotation>,
) {
    let Some(Json::List(citations)) = fields.get("citations") else {
        return;
    };
    for citation in citations {
        if let Some(annotation) = search_result_annotation(citation, block_offset) {
            annotations.push(annotation);
        }
    }
}

fn search_result_annotation(citation: &Json, block_offset: i128) -> Option<CitationAnnotation> {
    let Json::Object(entry) = citation else {
        return None;
    };
    let Some(Json::Object(location)) = entry.get("location") else {
        return None;
    };
    let Some(Json::Object(search)) = location.get("searchResultLocation") else {
        return None;
    };
    let start = integer(search.get("start"))?;
    let end = integer(search.get("end"))?;
    let start_index = block_offset
        .checked_add(start)
        .and_then(Number::from_i128)?;
    let end_index = block_offset.checked_add(end).and_then(Number::from_i128)?;
    Some(CitationAnnotation {
        kind: "url_citation".to_owned(),
        url_citation: UrlCitation {
            start_index,
            end_index,
            title: citation_text(entry.get("title")),
            url: citation_text(entry.get("source")),
        },
    })
}

fn citation_block_text(fields: &IndexMap<String, Json>) -> String {
    let Some(Json::List(parts)) = fields.get("content") else {
        return String::new();
    };
    let mut text = String::new();
    for part in parts {
        if let Json::Object(entry) = part
            && let Some(Json::Text(value)) = entry.get("text")
        {
            text.push_str(value);
        }
    }
    text
}

fn integer(value: Option<&Json>) -> Option<i128> {
    match value {
        Some(Json::Bool(flag)) => Some(i128::from(*flag)),
        Some(Json::Number(number)) => number.to_string().parse::<i128>().ok(),
        Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) | None => None,
    }
}

// LiteLLM stringifies whatever the citation carries; only the declared str value survives here.
fn citation_text(value: Option<&Json>) -> String {
    match value {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    }
}
