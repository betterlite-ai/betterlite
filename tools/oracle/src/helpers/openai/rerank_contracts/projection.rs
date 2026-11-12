use std::sync::PoisonError;

use betterlite_error::facts::presence::Field;
use betterlite_models::decisions::Json;
use betterlite_provider::api::rerank_contracts::{
    OptionalRerankParams, ReturnDocuments, get_optional_rerank_params,
};
use serde_json::{Value, json};

use super::recording::{Recording, object_of};
use super::{ContractError, ProjectionParams};

pub(super) fn run(params: &ProjectionParams) -> Result<Value, ContractError> {
    let mut reports = Vec::new();
    for call in &params.calls {
        let mut nondefault = call.nondefault.clone();
        let return_documents = match &call.return_documents {
            Field::Missing => ReturnDocuments::Default,
            Field::Null => ReturnDocuments::Null,
            Field::Value(value) => ReturnDocuments::Value(*value),
        };
        let config = Recording::default();
        let returned = get_optional_rerank_params(
            &config,
            OptionalRerankParams {
                non_default_params: nondefault.as_mut(),
                model: &call.model,
                drop_params: call.drop_params,
                query: &call.query,
                documents: &call.documents,
                custom_llm_provider: call.custom_llm_provider.as_deref(),
                top_n: call.top_n,
                rank_fields: call.rank_fields.as_deref(),
                return_documents,
                max_chunks_per_doc: call.max_chunks_per_doc,
                max_tokens_per_doc: call.max_tokens_per_doc,
                instruction: call.instruction.as_deref(),
            },
        )?;
        let mapper = config
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned();
        reports.push(json!({
            "mapper": mapper.unwrap_or(Json::Null),
            "nondefault_after": nondefault.as_ref().map_or(Json::Null, object_of),
            "returned": object_of(&returned),
        }));
    }
    Ok(json!({"calls": reports}))
}
