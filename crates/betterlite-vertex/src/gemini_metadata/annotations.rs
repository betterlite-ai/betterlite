use betterlite_models::decisions::Json;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UrlCitation {
    pub start_index: i64,
    pub end_index: i64,
    pub url: String,
    pub title: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationType {
    UrlCitation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GroundingAnnotation {
    #[serde(rename = "type")]
    pub kind: AnnotationType,
    pub url_citation: UrlCitation,
}

// LiteLLM ignores the answer text here, so the offsets stay exactly as the provider sent them
#[must_use]
pub fn grounding_annotations(grounding_metadata: &[Json]) -> Vec<GroundingAnnotation> {
    let mut annotations = Vec::new();
    for metadata in grounding_metadata {
        let web_chunks = web_chunks(metadata);
        let supports = object_get(metadata, "groundingSupports")
            .and_then(as_list)
            .unwrap_or_default();
        annotations.extend(
            supports
                .iter()
                .filter_map(|support| support_annotation(support, &web_chunks)),
        );
    }
    annotations
}

fn web_chunks(metadata: &Json) -> Vec<(i64, &Json)> {
    let mut chunks = Vec::new();
    for (position, chunk) in object_get(metadata, "groundingChunks")
        .and_then(as_list)
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let Some(web @ Json::Object(_)) = object_get(chunk, "web") else {
            continue;
        };
        if let Ok(index) = i64::try_from(position) {
            chunks.push((index, web));
        }
    }
    chunks
}

fn support_annotation(support: &Json, web_chunks: &[(i64, &Json)]) -> Option<GroundingAnnotation> {
    let segment = object_get(support, "segment")?;
    let start_index = integer(object_get(segment, "startIndex")?)?;
    let end_index = integer(object_get(segment, "endIndex")?)?;
    let indices = object_get(support, "groundingChunkIndices")
        .and_then(as_list)
        .unwrap_or_default();
    let first_chunk = integer(indices.first()?)?;
    let (_, web) = web_chunks.iter().find(|(index, _)| *index == first_chunk)?;
    Some(GroundingAnnotation {
        kind: AnnotationType::UrlCitation,
        url_citation: UrlCitation {
            start_index,
            end_index,
            url: object_get(web, "uri")
                .and_then(text_of)
                .unwrap_or_default()
                .to_owned(),
            title: object_get(web, "title")
                .and_then(text_of)
                .unwrap_or_default()
                .to_owned(),
        },
    })
}

fn object_get<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    let Json::Object(fields) = value else {
        return None;
    };
    fields.get(name)
}

fn as_list(value: &Json) -> Option<&[Json]> {
    match value {
        Json::List(items) => Some(items),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

fn integer(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn text_of(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
