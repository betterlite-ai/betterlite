use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::LoggingState;
use super::LoggingStateError;
use super::pricing::takes_custom_pricing;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CallUpdate {
    pub model: Option<String>,
    pub user: Option<String>,
    pub optional_params: IndexMap<String, Json>,
    pub additional_params: IndexMap<String, Json>,
}

pub fn update_environment_variables(
    state: &mut LoggingState,
    litellm_params: &IndexMap<String, Json>,
    update: CallUpdate,
) -> Result<(), LoggingStateError> {
    state.optional_params = update.optional_params;
    if let Some(model) = update.model {
        state.model = model;
    }
    state.user = update.user;
    state.litellm_params.extend(
        litellm_params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    state.litellm_request_debug = matches!(
        litellm_params.get("litellm_request_debug"),
        Some(Json::Bool(true))
    );
    if takes_custom_pricing(litellm_params) {
        state.cost.custom_pricing = true;
    }
    record_call_environment(state, &update.additional_params)
}

pub fn update_from_kwargs(
    state: &mut LoggingState,
    kwargs: &IndexMap<String, Json>,
    litellm_params: Option<&IndexMap<String, Json>>,
    update: CallUpdate,
) -> Result<(), LoggingStateError> {
    update_environment_variables(
        state,
        &merge_kwargs_litellm_params(kwargs, litellm_params),
        update,
    )
}

pub fn update_messages(state: &mut LoggingState, messages: Vec<Json>) {
    state.messages = Some(messages.clone());
    state
        .model_call_details
        .insert("messages".to_owned(), Json::List(messages));
}

fn record_call_environment(
    state: &mut LoggingState,
    additional_params: &IndexMap<String, Json>,
) -> Result<(), LoggingStateError> {
    insert_call_identity(state);
    insert_call_environment(state);
    apply_parameters(state, additional_params);
    apply_custom_llm_provider(state)
}

fn insert_call_identity(state: &mut LoggingState) {
    state
        .model_call_details
        .insert("model".to_owned(), Json::Text(state.model.clone()));
    state
        .model_call_details
        .insert("call_type".to_owned(), Json::Text(state.call_type.clone()));
    state.model_call_details.insert(
        "litellm_call_id".to_owned(),
        Json::Text(state.litellm_call_id.clone()),
    );
}

fn insert_call_environment(state: &mut LoggingState) {
    state.model_call_details.insert(
        "messages".to_owned(),
        state
            .messages
            .as_ref()
            .map_or(Json::Null, |messages| Json::List(messages.clone())),
    );
    state.model_call_details.insert(
        "optional_params".to_owned(),
        Json::Object(state.optional_params.clone()),
    );
    state.model_call_details.insert(
        "litellm_params".to_owned(),
        Json::Object(state.litellm_params.clone()),
    );
    state.model_call_details.insert(
        "start_time".to_owned(),
        Json::Text(state.start_time.to_rfc3339()),
    );
    state.model_call_details.insert(
        "stream".to_owned(),
        state.stream.map_or(Json::Null, Json::Bool),
    );
    state.model_call_details.insert(
        "user".to_owned(),
        state
            .user
            .as_ref()
            .map_or(Json::Null, |user| Json::Text(user.clone())),
    );
    state.model_call_details.insert(
        "completion_start_time".to_owned(),
        state
            .completion_start_time
            .map_or(Json::Null, |time| Json::Text(time.to_rfc3339())),
    );
    state.model_call_details.insert(
        "callback_dynamic_params".to_owned(),
        Json::Object(state.callback_dynamic_params.clone()),
    );
}

fn apply_parameters(state: &mut LoggingState, additional_params: &IndexMap<String, Json>) {
    for (name, value) in &state.optional_params {
        state.model_call_details.insert(name.clone(), value.clone());
    }
    for (name, value) in additional_params {
        state.model_call_details.insert(name.clone(), value.clone());
    }
    if let Some(stream_options) = additional_params.get("stream_options") {
        state.stream_options = Some(stream_options.clone());
    }
}

fn apply_custom_llm_provider(state: &mut LoggingState) -> Result<(), LoggingStateError> {
    let Some(recorded) = state.model_call_details.get("custom_llm_provider").cloned() else {
        return Ok(());
    };
    match recorded {
        Json::Null => state.custom_llm_provider = None,
        Json::Text(name) => state.custom_llm_provider = Some(name),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(LoggingStateError::FieldType {
                field: "custom_llm_provider",
            });
        }
    }
    Ok(())
}

// LiteLLM merges kwargs metadata under litellm_metadata and lets neither override caller litellm_params metadata
fn merge_kwargs_litellm_params(
    kwargs: &IndexMap<String, Json>,
    litellm_params: Option<&IndexMap<String, Json>>,
) -> IndexMap<String, Json> {
    let mut merged = metadata_slots(kwargs);
    let Some(caller_params) = litellm_params else {
        return merged;
    };
    let caller_metadata = caller_metadata(caller_params);
    for (name, value) in caller_params {
        if name != "metadata" {
            merged.insert(name.clone(), value.clone());
        }
    }
    if let Some(pending) = caller_metadata.filter(|metadata| !metadata.is_empty()) {
        merge_missing_metadata(&mut merged, &pending);
    }
    merged
}

fn metadata_slots(kwargs: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut merged = IndexMap::new();
    if let Some(Json::Object(metadata)) = kwargs.get("metadata") {
        merged.insert("metadata".to_owned(), Json::Object(metadata.clone()));
    }
    if let Some(Json::Object(litellm_metadata)) = kwargs.get("litellm_metadata") {
        merged.insert(
            "litellm_metadata".to_owned(),
            Json::Object(litellm_metadata.clone()),
        );
        if !merged.contains_key("metadata") {
            merged.insert(
                "metadata".to_owned(),
                Json::Object(litellm_metadata.clone()),
            );
        }
    }
    merged
}

fn caller_metadata(litellm_params: &IndexMap<String, Json>) -> Option<IndexMap<String, Json>> {
    match litellm_params.get("metadata") {
        Some(Json::Object(metadata)) => Some(metadata.clone()),
        _ => None,
    }
}

fn merge_missing_metadata(
    merged: &mut IndexMap<String, Json>,
    caller_metadata: &IndexMap<String, Json>,
) {
    let slot = merged
        .entry("metadata".to_owned())
        .or_insert_with(|| Json::Object(IndexMap::new()));
    let Json::Object(existing) = slot else {
        return;
    };
    for (name, value) in caller_metadata {
        if !existing.contains_key(name) {
            existing.insert(name.clone(), value.clone());
        }
    }
}
