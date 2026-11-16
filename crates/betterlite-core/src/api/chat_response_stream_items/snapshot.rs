use betterlite_models::decisions::Json;
use betterlite_streaming::streamed_response_assembly::AssemblyError;
use betterlite_streaming::streamed_response_assembly::response::Response;
use indexmap::IndexMap;

use crate::api::chat_response_stream_tools::state::ChatStreamState;

use super::error::StreamItemsError;

// What the streaming driver's logging context supplies to the end-of-stream assembly.
pub trait StreamAssembly {
    fn assemble(
        &self,
        chunks: &[IndexMap<String, Json>],
    ) -> Result<Option<Response>, AssemblyError>;
}

pub fn merge_provider_specific_fields<Context>(
    state: &mut ChatStreamState<Context>,
    provider_fields: &IndexMap<String, Json>,
) {
    for (key, value) in provider_fields {
        state
            .progress
            .provider_specific_fields
            .insert(key.clone(), value.clone());
    }
}

#[must_use]
pub fn snapshot_chunk(chunk: &IndexMap<String, Json>) -> Json {
    let mut snapshot = chunk.clone();
    // A chunk never given hidden parameters keeps ModelResponseStream's empty dict; an explicit null drops the key.
    let hidden = match snapshot.shift_remove("_hidden_params") {
        None => Some(Json::Object(IndexMap::new())),
        Some(Json::Null) => None,
        Some(value) => Some(value),
    };
    if let Some(hidden_value) = &hidden
        && matches!(snapshot.get("usage"), None | Some(Json::Null))
        && let Json::Object(hidden_fields) = hidden_value
        && let Some(usage) = hidden_fields
            .get("usage")
            .filter(|value| !matches!(value, Json::Null))
    {
        snapshot.insert("usage".to_owned(), usage.clone());
    }
    if let Some(hidden_value) = hidden {
        snapshot.insert("_hidden_params".to_owned(), hidden_value);
    }
    Json::Object(snapshot)
}

pub fn create_litellm_model_response<Context>(
    state: &ChatStreamState<Context>,
) -> Result<Option<Json>, StreamItemsError>
where
    Context: StreamAssembly,
{
    let Some(response) = state
        .logging_context
        .assemble(&state.progress.chunk_snapshots)?
    else {
        return Ok(None);
    };
    let encoded = serde_json::to_vec(&response).map_err(AssemblyError::from)?;
    let mut assembled: IndexMap<String, Json> =
        serde_json::from_slice(&encoded).map_err(AssemblyError::from)?;
    merge_hidden_provider_fields(&mut assembled, &state.progress.provider_specific_fields)?;
    Ok(Some(Json::Object(assembled)))
}

pub(super) fn merge_hidden_provider_fields(
    assembled: &mut IndexMap<String, Json>,
    provider_fields: &IndexMap<String, Json>,
) -> Result<(), StreamItemsError> {
    if provider_fields.is_empty() {
        return Ok(());
    }
    if matches!(assembled.get("hidden_params"), None | Some(Json::Null)) {
        assembled.insert("hidden_params".to_owned(), Json::Object(IndexMap::new()));
    }
    let Some(Json::Object(hidden_fields)) = assembled.get_mut("hidden_params") else {
        return Err(StreamItemsError::HiddenParamsNotObject);
    };
    if matches!(
        hidden_fields.get("provider_specific_fields"),
        None | Some(Json::Null)
    ) {
        hidden_fields.insert(
            "provider_specific_fields".to_owned(),
            Json::Object(IndexMap::new()),
        );
    }
    let Some(Json::Object(merged)) = hidden_fields.get_mut("provider_specific_fields") else {
        return Err(StreamItemsError::ProviderFieldsNotObject);
    };
    for (key, value) in provider_fields {
        merged.insert(key.clone(), value.clone());
    }
    Ok(())
}
