use axum::http::StatusCode;

mod tests;

const AVAILABLE_MODELS_HINT: &str = "Call `/v1/models` to view available models for your key.";

// LiteLLM's ROUTE_ENDPOINT_MAPPING, one entry per route type in its order
const ROUTE_ENDPOINT_MAPPING: [(&str, &str); 66] = [
    ("acompletion", "/chat/completions"),
    ("atext_completion", "/completions"),
    ("aembedding", "/embeddings"),
    ("aimage_generation", "/image/generations"),
    ("aspeech", "/audio/speech"),
    ("atranscription", "/audio/transcriptions"),
    ("amoderation", "/moderations"),
    ("arerank", "/rerank"),
    ("aresponses", "/responses"),
    ("_aresponses_websocket", "/responses"),
    ("alist_input_items", "/responses/{response_id}/input_items"),
    ("aimage_edit", "/images/edits"),
    ("acancel_responses", "/responses/{response_id}/cancel"),
    ("acompact_responses", "/responses/compact"),
    ("aocr", "/ocr"),
    ("asearch", "/search"),
    ("adecisions", "/decisions"),
    ("asystemone", "/systemone"),
    ("avideo_generation", "/videos"),
    ("avideo_list", "/videos"),
    ("avideo_status", "/videos/{video_id}"),
    ("avideo_content", "/videos/{video_id}/content"),
    ("avideo_remix", "/videos/{video_id}/remix"),
    ("avideo_create_character", "/videos/characters"),
    ("avideo_get_character", "/videos/characters/{character_id}"),
    ("avideo_edit", "/videos/edits"),
    ("avideo_extension", "/videos/extensions"),
    ("acreate_realtime_client_secret", "/realtime/client_secrets"),
    ("arealtime_calls", "/realtime/calls"),
    (
        "acreate_realtime_transcription_session",
        "/realtime/transcription_sessions",
    ),
    ("acreate_container", "/containers"),
    ("alist_containers", "/containers"),
    ("aretrieve_container", "/containers/{container_id}"),
    ("adelete_container", "/containers/{container_id}"),
    ("aupload_container_file", "/containers/{container_id}/files"),
    ("alist_container_files", "/containers/{container_id}/files"),
    (
        "aretrieve_container_file",
        "/containers/{container_id}/files/{file_id}",
    ),
    (
        "adelete_container_file",
        "/containers/{container_id}/files/{file_id}",
    ),
    (
        "aretrieve_container_file_content",
        "/containers/{container_id}/files/{file_id}/content",
    ),
    ("acreate_skill", "/skills"),
    ("alist_skills", "/skills"),
    ("aget_skill", "/skills/{skill_id}"),
    ("adelete_skill", "/skills/{skill_id}"),
    ("aingest", "/rag/ingest"),
    ("acreate_interaction", "/interactions"),
    ("aget_interaction", "/interactions/{interaction_id}"),
    ("adelete_interaction", "/interactions/{interaction_id}"),
    (
        "acancel_interaction",
        "/interactions/{interaction_id}/cancel",
    ),
    ("acreate_agent", "/v1beta/agents"),
    ("alist_agents", "/v1beta/agents"),
    ("aget_agent", "/v1beta/agents/{name}"),
    ("adelete_agent", "/v1beta/agents/{name}"),
    ("alist_agent_versions", "/v1beta/agents/{name}/versions"),
    ("acreate_eval", "/evals"),
    ("alist_evals", "/evals"),
    ("aget_eval", "/evals/{eval_id}"),
    ("aupdate_eval", "/evals/{eval_id}"),
    ("adelete_eval", "/evals/{eval_id}"),
    ("acancel_eval", "/evals/{eval_id}/cancel"),
    ("acreate_run", "/evals/{eval_id}/runs"),
    ("alist_runs", "/evals/{eval_id}/runs"),
    ("aget_run", "/evals/{eval_id}/runs/{run_id}"),
    ("acancel_run", "/evals/{eval_id}/runs/{run_id}/cancel"),
    ("adelete_run", "/evals/{eval_id}/runs/{run_id}"),
    ("acreate_batch", "/batches"),
    ("aretrieve_batch", "/batches"),
];

#[must_use]
pub fn route_endpoint(route_type: &str) -> Option<&'static str> {
    ROUTE_ENDPOINT_MAPPING
        .iter()
        .find(|(route, _endpoint)| *route == route_type)
        .map(|(_route, endpoint)| *endpoint)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ModelNotFoundDetail {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", detail.error)]
pub struct ProxyModelNotFoundError {
    pub detail: ModelNotFoundDetail,
    pub spend_log_error_message: String,
    // ast-grep-ignore: no-long-names
    pub retryable_with_model_read_through: bool,
}

impl ProxyModelNotFoundError {
    pub const STATUS: StatusCode = StatusCode::BAD_REQUEST;

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn new(route: &str, model_name: &str, retryable_with_model_read_through: bool) -> Self {
        let detail = ModelNotFoundDetail {
            error: format!(
                "{route}: Invalid model name passed in model={model_name}. {AVAILABLE_MODELS_HINT}"
            ),
        };
        let spend_log_error_message =
            format!("{route}: Invalid model name passed in. {AVAILABLE_MODELS_HINT}");
        Self {
            detail,
            spend_log_error_message,
            // ast-grep-ignore: no-long-names
            retryable_with_model_read_through,
        }
    }
}
