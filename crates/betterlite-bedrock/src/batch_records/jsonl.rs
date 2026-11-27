use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;

use super::chat_body::to_chat_body;
use super::error::BatchRecordsError;
use super::families::BatchFamilies;
use super::params::map_openai_to_bedrock_params;
use super::record::{BedrockBatchRecord, OpenAiBatchRecord};
use super::resolve::resolve_record_model_and_provider;
use crate::batch_embeddings::classify::BatchRecordKind;
use crate::batch_embeddings::mapping::{EmbeddingRequest, map_embedding_to_bedrock};
use crate::bedrock_model_capabilities::ModelInfoLookup;

const MODEL_FIELD: &str = "model";

// The record-time sources that live above this leaf: the alias map, the provider passes and the model info.
pub struct BatchRecordContext<'source, Lookup> {
    pub aliases: &'source BTreeMap<String, String>,
    pub families: &'source dyn BatchFamilies,
    pub lookup: &'source Lookup,
    pub resolution: &'source ResolutionContext<'source>,
}

// _transform_openai_jsonl_content_to_bedrock_jsonl_content: one Bedrock record per OpenAI record.
pub async fn transform_openai_jsonl<Lookup>(
    records: &[OpenAiBatchRecord],
    target_model: &str,
    context: &BatchRecordContext<'_, Lookup>,
) -> Result<Vec<BedrockBatchRecord>, BatchRecordsError>
where
    Lookup: ModelInfoLookup,
{
    let mut transformed = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let Json::Object(fields) = &record.body else {
            return Err(BatchRecordsError::RecordShape);
        };
        let record_model = body_model(fields)?;
        let (model, provider) = resolve_record_model_and_provider(
            record_model,
            target_model,
            context.aliases,
            context.resolution,
        );
        let model_input = if record.kind == BatchRecordKind::Embedding {
            embedding_model_input(&record.body, model, context.lookup).await?
        } else {
            let chat = to_chat_body(&record.body, record.kind, context.families).await?;
            map_openai_to_bedrock_params(
                &chat,
                model,
                provider,
                context.families,
                context.resolution,
            )
            .await?
        };
        transformed.push(BedrockBatchRecord {
            record_id: record.record_id(index),
            model_input,
        });
    }
    Ok(transformed)
}

async fn embedding_model_input<Lookup>(
    body: &Json,
    model: &str,
    lookup: &Lookup,
) -> Result<Json, BatchRecordsError>
where
    Lookup: ModelInfoLookup,
{
    let value = serde_json::to_value(body).map_err(BatchRecordsError::Wire)?;
    let request: EmbeddingRequest =
        serde_json::from_value(value).map_err(BatchRecordsError::Json)?;
    let mapped = map_embedding_to_bedrock(&request, model, lookup).await?;
    let value = serde_json::to_value(mapped).map_err(BatchRecordsError::Wire)?;
    serde_json::from_value(value).map_err(BatchRecordsError::Wire)
}

// `openai_body.get("model", "")`: an absent model names nothing.
fn body_model(fields: &IndexMap<String, Json>) -> Result<&str, BatchRecordsError> {
    match fields.get(MODEL_FIELD) {
        Some(Json::Text(model)) => Ok(model),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BatchRecordsError::Field { field: MODEL_FIELD })
        }
        None => Ok(""),
    }
}
