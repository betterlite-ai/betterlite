use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::NvidiaNimRerankError;
use crate::api::rerank_contracts::Params;

const MODEL: &str = "model";
const QUERY: &str = "query";
const DOCUMENTS: &str = "documents";
const PASSAGES: &str = "passages";
const TOP_K: &str = "top_k";
const TRUNCATE: &str = "truncate";
const TEXT: &str = "text";
const NONE: &str = "NONE";
const END: &str = "END";

pub fn rerank_body(
    model: &str,
    optional_rerank_params: &Params,
    supported_fields: &[&'static str],
) -> Result<Params, NvidiaNimRerankError> {
    let Some(query) = optional_rerank_params.get(QUERY) else {
        return Err(NvidiaNimRerankError::MissingQuery);
    };
    let Some(documents) = optional_rerank_params.get(DOCUMENTS) else {
        return Err(NvidiaNimRerankError::MissingDocuments);
    };
    let Json::List(documents) = documents else {
        return Err(NvidiaNimRerankError::DocumentsNotAList);
    };
    let passages = passages(documents, supported_fields)?;
    let mut body = Params::from([
        (MODEL.to_owned(), Json::Text(model.replace('_', "."))),
        (
            QUERY.to_owned(),
            Json::Object(IndexMap::from([(TEXT.to_owned(), query.clone())])),
        ),
        (PASSAGES.to_owned(), Json::List(passages)),
    ]);
    if let Some(top_k) = optional_rerank_params
        .get(TOP_K)
        .filter(|value| !matches!(value, Json::Null))
    {
        body.insert(TOP_K.to_owned(), top_k.clone());
    }
    if let Some(truncate) = optional_rerank_params.get(TRUNCATE)
        && matches!(truncate, Json::Text(text) if text == NONE || text == END)
    {
        body.insert(TRUNCATE.to_owned(), truncate.clone());
    }
    Ok(body)
}

fn passages(
    documents: &[Json],
    supported_fields: &[&'static str],
) -> Result<Vec<Json>, NvidiaNimRerankError> {
    let mut passages = Vec::with_capacity(documents.len());
    for (position, document) in documents.iter().enumerate() {
        passages.push(passage(document, supported_fields, position)?);
    }
    Ok(passages)
}

fn passage(
    document: &Json,
    supported_fields: &[&'static str],
    position: usize,
) -> Result<Json, NvidiaNimRerankError> {
    match document {
        Json::Text(text) => Ok(Json::Object(IndexMap::from([(
            TEXT.to_owned(),
            Json::Text(text.clone()),
        )]))),
        Json::Object(fields) => {
            let mut kept = IndexMap::new();
            for field in supported_fields {
                if let Some(value) = fields.get(*field) {
                    kept.insert((*field).to_owned(), value.clone());
                }
            }
            if kept.is_empty() {
                let encoded = serde_json::to_string(document)
                    .map_err(|source| NvidiaNimRerankError::Encode { source })?;
                Ok(Json::Object(IndexMap::from([(
                    TEXT.to_owned(),
                    Json::Text(encoded),
                )])))
            } else {
                Ok(Json::Object(kept))
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            Err(NvidiaNimRerankError::UnsupportedDocument { position })
        }
    }
}
