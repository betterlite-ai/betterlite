use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::CohereV2Error;

const DOCUMENT: &str = "document";
const URL_CITATION: &str = "url_citation";
const UNKNOWN_SOURCE: &str = "unknown";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Annotation {
    #[serde(rename = "type")]
    pub kind: String,
    pub url_citation: UrlCitation,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct UrlCitation {
    pub start_index: i64,
    pub end_index: i64,
    pub title: String,
    pub url: String,
}

pub fn annotations(citations: &[Json]) -> Result<Vec<Annotation>, CohereV2Error> {
    let mut found = Vec::new();
    for entry in citations {
        found.extend(citation_annotations(entry)?);
    }
    Ok(found)
}

fn citation_annotations(entry: &Json) -> Result<Vec<Annotation>, CohereV2Error> {
    let Json::Object(citation) = entry else {
        return Err(CohereV2Error::NotAnObject {
            field: "message.citations",
        });
    };
    let start_index = super::integer(citation.get("start"), "message.citations.start")?;
    let end_index = super::integer(citation.get("end"), "message.citations.end")?;
    let Some(Json::List(sources)) = citation.get("sources").filter(|value| value.is_truthy())
    else {
        return Ok(Vec::new());
    };
    let mut found = Vec::with_capacity(sources.len());
    for item in sources {
        if let Some(annotation) = source_annotation(item, start_index, end_index)? {
            found.push(annotation);
        }
    }
    Ok(found)
}

fn source_annotation(
    entry: &Json,
    start_index: i64,
    end_index: i64,
) -> Result<Option<Annotation>, CohereV2Error> {
    let Json::Object(source) = entry else {
        return Err(CohereV2Error::NotAnObject {
            field: "message.citations.sources",
        });
    };
    if source.get("type").and_then(Json::as_str) != Some(DOCUMENT) {
        return Ok(None);
    }
    let Some(document) = source.get("document") else {
        return Ok(None);
    };
    let Json::Object(fields) = document else {
        return Err(CohereV2Error::NotAnObject {
            field: "message.citations.sources.document",
        });
    };
    Ok(Some(Annotation {
        kind: URL_CITATION.to_owned(),
        url_citation: UrlCitation {
            start_index,
            end_index,
            title: document_title(fields)?,
            url: source_url(source)?,
        },
    }))
}

fn document_title(document: &Object) -> Result<String, CohereV2Error> {
    match document.get("title") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(CohereV2Error::NotText {
            field: "message.citations.sources.document.title",
        }),
    }
}

fn source_url(source: &Object) -> Result<String, CohereV2Error> {
    match source.get("url").filter(|value| value.is_truthy()) {
        Some(Json::Text(url)) => Ok(url.clone()),
        Some(_) => Err(CohereV2Error::NotText {
            field: "message.citations.sources.url",
        }),
        None => Ok(match source.get("id") {
            Some(Json::Text(id)) => format!("source:{id}"),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => format!("source:{UNKNOWN_SOURCE}"),
        }),
    }
}
