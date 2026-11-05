use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    AutorouterClassifier,
    AutorouterCompaction,
    ShadowEvalRouter,
    ShadowEvalJudge,
    LlmAsAJudgeGuardrail,
    BackgroundResponseCostPoll,
}

#[derive(Debug, Clone, Copy)]
pub enum Forwarding {
    Attached,
    Detached,
}

const IDENTITY_FIELDS: [&str; 8] = [
    "user_api_key",
    "user_api_key_hash",
    "user_api_key_alias",
    "user_api_key_team_id",
    "user_api_key_org_id",
    "user_api_key_user_id",
    "user_api_key_end_user_id",
    "user_api_key_auth",
];

pub fn forward(
    parent: &IndexMap<String, Json>,
    origin: Origin,
    mode: Forwarding,
) -> Result<IndexMap<String, Json>, super::CallbackMetadataError> {
    let mut output = parent.clone();
    match mode {
        Forwarding::Attached if parent.is_empty() => return Ok(output),
        Forwarding::Attached => {}
        Forwarding::Detached => output.retain(|key, _| IDENTITY_FIELDS.contains(&key.as_str())),
    }
    output.shift_remove("user_api_key_budget_reservation");
    if let Some(Json::Object(auth)) = output.get_mut("user_api_key_auth") {
        auth.shift_remove("budget_reservation");
    }
    let encoded = serde_json::to_vec(&origin)?;
    output.insert(
        "internal_call_origin".to_owned(),
        serde_json::from_slice(&encoded)?,
    );
    Ok(output)
}

#[derive(Default, Deserialize, Serialize)]
pub struct Session {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub litellm_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub litellm_trace_id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum CallCategory {
    Inference,
    Management,
}

#[must_use]
pub fn call_category(call_type: Option<&str>) -> CallCategory {
    match call_type {
        Some(
            "get_responses"
            | "aget_responses"
            | "delete_responses"
            | "adelete_responses"
            | "cancel_responses"
            | "acancel_responses"
            | "list_input_items"
            | "alist_input_items"
            | "vector_store_create"
            | "avector_store_create"
            | "vector_store_retrieve"
            | "avector_store_retrieve"
            | "vector_store_list"
            | "avector_store_list"
            | "vector_store_update"
            | "avector_store_update"
            | "vector_store_delete"
            | "avector_store_delete"
            | "vector_store_file_create"
            | "avector_store_file_create"
            | "vector_store_file_list"
            | "avector_store_file_list"
            | "vector_store_file_retrieve"
            | "avector_store_file_retrieve"
            | "vector_store_file_content"
            | "avector_store_file_content"
            | "vector_store_file_update"
            | "avector_store_file_update"
            | "vector_store_file_delete"
            | "avector_store_file_delete",
        ) => CallCategory::Management,
        Some(_) | None => CallCategory::Inference,
    }
}

#[must_use]
pub const fn is_unbilled(category: CallCategory, origin: Option<Origin>, background: bool) -> bool {
    match category {
        CallCategory::Inference => false,
        CallCategory::Management => {
            !background && !matches!(origin, Some(Origin::BackgroundResponseCostPoll))
        }
    }
}
