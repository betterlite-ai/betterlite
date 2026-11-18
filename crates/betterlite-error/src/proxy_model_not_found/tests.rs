#![cfg(test)]

use axum::http::StatusCode;
use rstest::rstest;

use super::{ProxyModelNotFoundError, route_endpoint};

#[rstest]
#[case::acompletion("acompletion", Some("/chat/completions"))]
#[case::atext_completion("atext_completion", Some("/completions"))]
#[case::aembedding("aembedding", Some("/embeddings"))]
#[case::aimage_generation("aimage_generation", Some("/image/generations"))]
#[case::aspeech("aspeech", Some("/audio/speech"))]
#[case::atranscription("atranscription", Some("/audio/transcriptions"))]
#[case::amoderation("amoderation", Some("/moderations"))]
#[case::arerank("arerank", Some("/rerank"))]
#[case::aresponses("aresponses", Some("/responses"))]
#[case::aresponses_websocket("_aresponses_websocket", Some("/responses"))]
#[case::alist_input_items("alist_input_items", Some("/responses/{response_id}/input_items"))]
#[case::aimage_edit("aimage_edit", Some("/images/edits"))]
#[case::acancel_responses("acancel_responses", Some("/responses/{response_id}/cancel"))]
#[case::acompact_responses("acompact_responses", Some("/responses/compact"))]
#[case::aocr("aocr", Some("/ocr"))]
#[case::asearch("asearch", Some("/search"))]
#[case::adecisions("adecisions", Some("/decisions"))]
#[case::asystemone("asystemone", Some("/systemone"))]
#[case::avideo_generation("avideo_generation", Some("/videos"))]
#[case::avideo_list("avideo_list", Some("/videos"))]
#[case::avideo_status("avideo_status", Some("/videos/{video_id}"))]
#[case::avideo_content("avideo_content", Some("/videos/{video_id}/content"))]
#[case::avideo_remix("avideo_remix", Some("/videos/{video_id}/remix"))]
#[case::avideo_create_character("avideo_create_character", Some("/videos/characters"))]
#[case::avideo_get_character("avideo_get_character", Some("/videos/characters/{character_id}"))]
#[case::avideo_edit("avideo_edit", Some("/videos/edits"))]
#[case::avideo_extension("avideo_extension", Some("/videos/extensions"))]
#[case::acreate_realtime_client_secret(
    "acreate_realtime_client_secret",
    Some("/realtime/client_secrets")
)]
#[case::arealtime_calls("arealtime_calls", Some("/realtime/calls"))]
#[case::acreate_realtime_transcription_session(
    "acreate_realtime_transcription_session",
    Some("/realtime/transcription_sessions")
)]
#[case::acreate_container("acreate_container", Some("/containers"))]
#[case::alist_containers("alist_containers", Some("/containers"))]
#[case::aretrieve_container("aretrieve_container", Some("/containers/{container_id}"))]
#[case::adelete_container("adelete_container", Some("/containers/{container_id}"))]
#[case::aupload_container_file("aupload_container_file", Some("/containers/{container_id}/files"))]
#[case::alist_container_files("alist_container_files", Some("/containers/{container_id}/files"))]
#[case::aretrieve_container_file(
    "aretrieve_container_file",
    Some("/containers/{container_id}/files/{file_id}")
)]
#[case::adelete_container_file(
    "adelete_container_file",
    Some("/containers/{container_id}/files/{file_id}")
)]
#[case::aretrieve_container_file_content(
    "aretrieve_container_file_content",
    Some("/containers/{container_id}/files/{file_id}/content")
)]
#[case::acreate_skill("acreate_skill", Some("/skills"))]
#[case::alist_skills("alist_skills", Some("/skills"))]
#[case::aget_skill("aget_skill", Some("/skills/{skill_id}"))]
#[case::adelete_skill("adelete_skill", Some("/skills/{skill_id}"))]
#[case::aingest("aingest", Some("/rag/ingest"))]
#[case::acreate_interaction("acreate_interaction", Some("/interactions"))]
#[case::aget_interaction("aget_interaction", Some("/interactions/{interaction_id}"))]
#[case::adelete_interaction("adelete_interaction", Some("/interactions/{interaction_id}"))]
#[case::acancel_interaction("acancel_interaction", Some("/interactions/{interaction_id}/cancel"))]
#[case::acreate_agent("acreate_agent", Some("/v1beta/agents"))]
#[case::alist_agents("alist_agents", Some("/v1beta/agents"))]
#[case::aget_agent("aget_agent", Some("/v1beta/agents/{name}"))]
#[case::adelete_agent("adelete_agent", Some("/v1beta/agents/{name}"))]
#[case::alist_agent_versions("alist_agent_versions", Some("/v1beta/agents/{name}/versions"))]
#[case::acreate_eval("acreate_eval", Some("/evals"))]
#[case::alist_evals("alist_evals", Some("/evals"))]
#[case::aget_eval("aget_eval", Some("/evals/{eval_id}"))]
#[case::aupdate_eval("aupdate_eval", Some("/evals/{eval_id}"))]
#[case::adelete_eval("adelete_eval", Some("/evals/{eval_id}"))]
#[case::acancel_eval("acancel_eval", Some("/evals/{eval_id}/cancel"))]
#[case::acreate_run("acreate_run", Some("/evals/{eval_id}/runs"))]
#[case::alist_runs("alist_runs", Some("/evals/{eval_id}/runs"))]
#[case::aget_run("aget_run", Some("/evals/{eval_id}/runs/{run_id}"))]
#[case::acancel_run("acancel_run", Some("/evals/{eval_id}/runs/{run_id}/cancel"))]
#[case::adelete_run("adelete_run", Some("/evals/{eval_id}/runs/{run_id}"))]
#[case::acreate_batch("acreate_batch", Some("/batches"))]
#[case::aretrieve_batch("aretrieve_batch", Some("/batches"))]
fn route_endpoint_maps_each_litellm_route(
    #[case] route_type: &str,
    #[case] expected: Option<&'static str>,
) {
    assert_eq!(route_endpoint(route_type), expected);
}

#[rstest]
#[case::anthropic_messages("anthropic_messages")]
#[case::allm_passthrough_route("allm_passthrough_route")]
#[case::file_content("afile_content")]
#[case::empty("")]
fn route_endpoint_answers_none_outside_the_litellm_map(#[case] route_type: &str) {
    assert_eq!(route_endpoint(route_type), None);
}

#[rstest]
#[case::mapped_path("/chat/completions", "gpt-x")]
#[case::own_route_text("file operation", "file-abc123")]
#[case::route_type("anthropic_messages", "claude-sonnet")]
fn not_found_detail_and_spend_texts_carry_the_route(#[case] route: &str, #[case] model_name: &str) {
    let error = ProxyModelNotFoundError::new(route, model_name, true);
    assert_eq!(
        error.detail.error,
        format!(
            "{route}: Invalid model name passed in model={model_name}. Call `/v1/models` to view available models for your key."
        )
    );
    assert_eq!(
        error.spend_log_error_message,
        format!(
            "{route}: Invalid model name passed in. Call `/v1/models` to view available models for your key."
        )
    );
}

#[test]
fn not_found_empty_model_name_leaves_the_model_blank() {
    let error = ProxyModelNotFoundError::new("completion", "", true);
    assert_eq!(
        error.detail.error,
        "completion: Invalid model name passed in model=. Call `/v1/models` to view available models for your key."
    );
    assert_eq!(
        error.spend_log_error_message,
        "completion: Invalid model name passed in. Call `/v1/models` to view available models for your key."
    );
}

#[rstest]
#[case::retryable(true)]
#[case::not_retryable(false)]
fn not_found_retryable_flag_is_kept(#[case] retryable_with_model_read_through: bool) {
    let error = ProxyModelNotFoundError::new(
        "/chat/completions",
        "gpt-x",
        retryable_with_model_read_through,
    );
    assert_eq!(
        error.retryable_with_model_read_through,
        retryable_with_model_read_through
    );
}

#[test]
fn not_found_status_is_400() {
    assert_eq!(ProxyModelNotFoundError::STATUS, StatusCode::BAD_REQUEST);
    assert_eq!(ProxyModelNotFoundError::STATUS.as_u16(), 400);
}

#[test]
fn not_found_display_is_the_detail_error() {
    let error = ProxyModelNotFoundError::new("file operation", "file-abc123", false);
    assert_eq!(error.to_string(), error.detail.error);
}

#[test]
fn not_found_detail_serializes_as_the_error_object() {
    let error = ProxyModelNotFoundError::new("/chat/completions", "gpt-x", true);
    assert_eq!(
        serde_json::to_value(&error.detail).unwrap(),
        serde_json::json!({
            "error": "/chat/completions: Invalid model name passed in model=gpt-x. Call `/v1/models` to view available models for your key."
        })
    );
}
