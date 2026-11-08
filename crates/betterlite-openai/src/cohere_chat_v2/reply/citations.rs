use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::ReplyError;

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

pub fn annotations(citations: &[Json]) -> Result<Vec<Annotation>, ReplyError> {
    let groups = citations
        .iter()
        .map(citation_annotations)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(groups.concat())
}

fn citation_annotations(entry: &Json) -> Result<Vec<Annotation>, ReplyError> {
    let Json::Object(citation) = entry else {
        return Err(ReplyError::Field("message.citations", "is not an object"));
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
) -> Result<Option<Annotation>, ReplyError> {
    let Json::Object(source) = entry else {
        return Err(ReplyError::Field(
            "message.citations.sources",
            "is not an object",
        ));
    };
    if source.get("type").and_then(Json::as_str) != Some(DOCUMENT) {
        return Ok(None);
    }
    let Some(document) = source.get("document") else {
        return Ok(None);
    };
    let Json::Object(fields) = document else {
        return Err(ReplyError::Field(
            "message.citations.sources.document",
            "is not an object",
        ));
    };
    let title = match fields.get("title") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Text(text)) => text.clone(),
        Some(_) => {
            return Err(ReplyError::Field(
                "message.citations.sources.document.title",
                "is not a string",
            ));
        }
    };
    Ok(Some(Annotation {
        kind: URL_CITATION.to_owned(),
        url_citation: UrlCitation {
            start_index,
            end_index,
            title,
            url: source_url(source)?,
        },
    }))
}

fn source_url(source: &Object) -> Result<String, ReplyError> {
    match source.get("url").filter(|value| value.is_truthy()) {
        Some(Json::Text(url)) => Ok(url.clone()),
        Some(_) => Err(ReplyError::Field(
            "message.citations.sources.url",
            "is not a string",
        )),
        None => Ok(match source.get("id") {
            Some(Json::Text(id)) => format!("source:{id}"),
            Some(_) | None => format!("source:{UNKNOWN_SOURCE}"),
        }),
    }
}
