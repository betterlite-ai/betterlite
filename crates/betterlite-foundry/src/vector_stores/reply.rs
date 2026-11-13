use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreResultContent, VectorStoreSearchResponse, VectorStoreSearchResult,
};
use rust_decimal::Decimal;

use super::error::AzureSearchError;
use super::search::SearchLogging;

pub const SEARCH_RESULTS_OBJECT: &str = "vector_store.search_results.page";

const VALUE_FIELD: &str = "value";
const ID_FIELD: &str = "id";
const CONTENT_FIELD: &str = "content";
const SCORE_FIELD: &str = "@search.score";
const DOCUMENT_ID_FIELD: &str = "document_id";
// LiteLLM always drops the literal contentVector, whichever vector field the request named
const EXCLUDED_FIELDS: [&str; 4] = [ID_FIELD, CONTENT_FIELD, "contentVector", SCORE_FIELD];
const TEXT_KIND: &str = "text";

pub struct AzureSearchStoreReply<'reply> {
    pub status_code: u16,
    pub headers: &'reply [(String, String)],
    pub body: &'reply [u8],
    pub logging: &'reply SearchLogging,
}

pub fn decode_azure_search_reply(
    reply: &AzureSearchStoreReply<'_>,
) -> Result<VectorStoreSearchResponse, AzureSearchError> {
    let parsed: Json =
        serde_json::from_slice(reply.body).map_err(|source| AzureSearchError::ReplyJson {
            status_code: reply.status_code,
            headers: reply.headers.to_vec(),
            source,
        })?;
    let Json::Object(response) = parsed else {
        return Err(shape_error(reply, "the reply is not an object"));
    };
    let items: &[Json] = match response.get(VALUE_FIELD) {
        None => &[],
        Some(Json::List(items)) => items,
        Some(_) => return Err(shape_error(reply, "value is not an array")),
    };
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        results.push(search_result(reply, item)?);
    }
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_RESULTS_OBJECT.to_owned()),
        search_query: Some(search_query(reply.logging)),
        data: Some(results),
        extra: BTreeMap::new(),
    })
}

fn search_result(
    reply: &AzureSearchStoreReply<'_>,
    item: &Json,
) -> Result<VectorStoreSearchResult, AzureSearchError> {
    let Json::Object(fields) = item else {
        return Err(shape_error(reply, "a result is not an object"));
    };
    let document_id = text_field(reply, fields, ID_FIELD)?;
    let text = text_field(reply, fields, CONTENT_FIELD)?;
    let score = score_field(reply, fields)?;
    let mut attributes = fields.clone();
    for key in EXCLUDED_FIELDS {
        attributes.shift_remove(key);
    }
    attributes.insert(
        DOCUMENT_ID_FIELD.to_owned(),
        Json::Text(document_id.clone()),
    );
    Ok(VectorStoreSearchResult {
        score: Some(score),
        content: Some(vec![VectorStoreResultContent {
            text: Some(text),
            type_name: Some(TEXT_KIND.to_owned()),
            extra: BTreeMap::new(),
        }]),
        file_id: Some(document_id.clone()),
        filename: Some(format!("Document {document_id}")),
        attributes: Some(
            serde_json::to_value(Json::Object(attributes))
                .map_err(|source| AzureSearchError::Attributes { source })?,
        ),
        extra: BTreeMap::new(),
    })
}

// an absent or null field takes its default, which is the empty string for the id and the content
fn text_field(
    reply: &AzureSearchStoreReply<'_>,
    fields: &indexmap::IndexMap<String, Json>,
    name: &'static str,
) -> Result<String, AzureSearchError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(shape_error(reply, &format!("{name} is not a string"))),
    }
}

fn score_field(
    reply: &AzureSearchStoreReply<'_>,
    fields: &indexmap::IndexMap<String, Json>,
) -> Result<Score, AzureSearchError> {
    match fields.get(SCORE_FIELD) {
        None | Some(Json::Null) => Ok(Score(Decimal::ZERO)),
        Some(Json::Number(number)) => {
            Score::try_from(number.clone()).map_err(|source| AzureSearchError::Score { source })
        }
        Some(_) => Err(shape_error(reply, "@search.score is not a number")),
    }
}

fn search_query(logging: &SearchLogging) -> String {
    match logging.model_call_details.get("input") {
        Some(Json::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

fn shape_error(reply: &AzureSearchStoreReply<'_>, message: &str) -> AzureSearchError {
    AzureSearchError::ReplyShape {
        status_code: reply.status_code,
        headers: reply.headers.to_vec(),
        message: message.to_owned(),
    }
}
