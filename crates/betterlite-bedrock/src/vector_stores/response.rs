use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreResultContent, VectorStoreSearchResponse, VectorStoreSearchResult,
};
use indexmap::IndexMap;
use thiserror::Error;

use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput};
use crate::vector_stores::error::{BedrockFailure, VectorStoreError};
use crate::vector_stores::metadata::{file_id_from_metadata, filename_from_metadata, non_empty};

const SEARCH_RESULTS_OBJECT: &str = "vector_store.search_results.page";

#[derive(Debug, Error)]
enum ReplyError {
    #[error("the reply is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the reply is not an object")]
    NotObject,
    #[error("retrievalResults is not an array")]
    ResultsNotArray,
    #[error("retrievalResults[{index}] is not an object")]
    ResultNotObject { index: usize },
    #[error("retrievalResults[{index}].content is not an object")]
    ContentNotObject { index: usize },
    #[error("retrievalResults[{index}].metadata is not an object")]
    MetadataNotObject { index: usize },
}

pub fn decode_knowledge_base_reply(
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    search_query: &str,
) -> Result<VectorStoreSearchResponse, VectorStoreError> {
    let parsed = serde_json::from_slice::<Json>(body)
        .map_err(|source| provider_error(status, headers, &ReplyError::Json { source }))?;
    let Json::Object(response) = parsed else {
        return Err(provider_error(status, headers, &ReplyError::NotObject));
    };
    let items: &[Json] = match response.get("retrievalResults") {
        None | Some(Json::Null) => &[],
        Some(Json::List(items)) => items.as_slice(),
        Some(other) if !non_empty(other) => &[],
        Some(_) => {
            return Err(provider_error(
                status,
                headers,
                &ReplyError::ResultsNotArray,
            ));
        }
    };
    let mut results = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if let Some(result) = search_result(status, headers, index, item)? {
            results.push(result);
        }
    }
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_RESULTS_OBJECT.to_owned()),
        search_query: Some(search_query.to_owned()),
        data: Some(results),
        extra: BTreeMap::new(),
    })
}

fn search_result(
    status: u16,
    headers: &[(String, String)],
    index: usize,
    item: &Json,
) -> Result<Option<VectorStoreSearchResult>, VectorStoreError> {
    let Json::Object(fields) = item else {
        return Err(provider_error(
            status,
            headers,
            &ReplyError::ResultNotObject { index },
        ));
    };
    let Some(text) = content_text(status, headers, index, fields)? else {
        return Ok(None);
    };
    let empty = IndexMap::new();
    let metadata = metadata_of(status, headers, index, fields)?.unwrap_or(&empty);
    let content = build_content(text)?;
    let mut result = build_result(metadata, content)?;
    apply_score(&mut result, fields.get("score"))?;
    Ok(Some(result))
}

fn content_text<'item>(
    status: u16,
    headers: &[(String, String)],
    index: usize,
    item: &'item IndexMap<String, Json>,
) -> Result<Option<&'item Json>, VectorStoreError> {
    let Some(content) = item.get("content") else {
        return Ok(None);
    };
    if !non_empty(content) {
        return Ok(None);
    }
    let Json::Object(content_fields) = content else {
        return Err(provider_error(
            status,
            headers,
            &ReplyError::ContentNotObject { index },
        ));
    };
    Ok(content_fields
        .get("text")
        .filter(|text| !matches!(text, Json::Null)))
}

fn metadata_of<'item>(
    status: u16,
    headers: &[(String, String)],
    index: usize,
    item: &'item IndexMap<String, Json>,
) -> Result<Option<&'item IndexMap<String, Json>>, VectorStoreError> {
    match item.get("metadata") {
        Some(Json::Object(metadata)) => Ok(Some(metadata)),
        Some(other) if non_empty(other) => Err(provider_error(
            status,
            headers,
            &ReplyError::MetadataNotObject { index },
        )),
        None | Some(_) => Ok(None),
    }
}

fn build_content(text: &Json) -> Result<VectorStoreResultContent, VectorStoreError> {
    let mut content = VectorStoreResultContent {
        text: None,
        type_name: Some("text".to_owned()),
        extra: BTreeMap::new(),
    };
    if let Json::Text(value) = text {
        content.text = Some(value.clone());
    } else {
        content
            .extra
            .insert("text".to_owned(), serde_json::to_value(text)?);
    }
    Ok(content)
}

fn build_result(
    metadata: &IndexMap<String, Json>,
    content: VectorStoreResultContent,
) -> Result<VectorStoreSearchResult, VectorStoreError> {
    let mut result = VectorStoreSearchResult {
        score: None,
        content: Some(vec![content]),
        file_id: None,
        filename: None,
        attributes: Some(serde_json::to_value(Json::Object(metadata.clone()))?),
        extra: BTreeMap::new(),
    };
    match file_id_from_metadata(metadata) {
        Json::Text(value) => result.file_id = Some(value),
        other
        @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            result
                .extra
                .insert("file_id".to_owned(), serde_json::to_value(&other)?);
        }
    }
    match filename_from_metadata(metadata) {
        Json::Text(value) => result.filename = Some(value),
        other
        @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            result
                .extra
                .insert("filename".to_owned(), serde_json::to_value(&other)?);
        }
    }
    Ok(result)
}

fn apply_score(
    result: &mut VectorStoreSearchResult,
    score: Option<&Json>,
) -> Result<(), VectorStoreError> {
    match score {
        Some(Json::Number(number)) => match Score::try_from(number.clone()) {
            Ok(parsed) => result.score = Some(parsed),
            Err(_) => {
                result
                    .extra
                    .insert("score".to_owned(), serde_json::to_value(number)?);
            }
        },
        other => {
            result.extra.insert(
                "score".to_owned(),
                serde_json::to_value(other.unwrap_or(&Json::Null))?,
            );
        }
    }
    Ok(())
}

fn provider_error(
    status: u16,
    headers: &[(String, String)],
    cause: &ReplyError,
) -> VectorStoreError {
    let input = ErrorInput {
        status_code: status,
        message: cause.to_string(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers.to_vec())),
        response: None,
    };
    match BedrockError::try_from(input) {
        Ok(error) => VectorStoreError::Provider {
            status,
            error: BedrockFailure {
                cause: Box::new(error),
            },
        },
        Err(source) => VectorStoreError::Headers { source },
    }
}
