#![cfg(test)]

use rstest::rstest;

use super::{ANTHROPIC_ROUTES, OPENAI_ROUTES, llm_api_routes};
use crate::auxiliary_routes;

#[rstest]
#[case::engine_chat_completions(0, "/engines/{model}/chat/completions")]
#[case::deployment_chat_completions(1, "/openai/deployments/{model}/chat/completions")]
#[case::chat_completions(2, "/chat/completions")]
#[case::v1_chat_completions(3, "/v1/chat/completions")]
#[case::cursor_chat_completions(4, "/cursor/chat/completions")]
#[case::cursor_models(5, "/cursor/models")]
#[case::cursor_v1_models(6, "/cursor/v1/models")]
#[case::engine_completions(7, "/engines/{model}/completions")]
#[case::deployment_completions(8, "/openai/deployments/{model}/completions")]
#[case::completions(9, "/completions")]
#[case::v1_completions(10, "/v1/completions")]
#[case::engine_embeddings(11, "/engines/{model}/embeddings")]
#[case::deployment_embeddings(12, "/openai/deployments/{model}/embeddings")]
#[case::embeddings(13, "/embeddings")]
#[case::v1_embeddings(14, "/v1/embeddings")]
#[case::images_generations(15, "/images/generations")]
#[case::v1_images_generations(16, "/v1/images/generations")]
#[case::images_edits(17, "/images/edits")]
#[case::v1_images_edits(18, "/v1/images/edits")]
#[case::videos(19, "/videos")]
#[case::v1_videos(20, "/v1/videos")]
#[case::videos_item(21, "/videos/{video_id}")]
#[case::v1_videos_item(22, "/v1/videos/{video_id}")]
#[case::videos_item_content(23, "/videos/{video_id}/content")]
#[case::v1_videos_item_content(24, "/v1/videos/{video_id}/content")]
#[case::videos_item_remix(25, "/videos/{video_id}/remix")]
#[case::v1_videos_item_remix(26, "/v1/videos/{video_id}/remix")]
#[case::audio_transcriptions(27, "/audio/transcriptions")]
#[case::v1_audio_transcriptions(28, "/v1/audio/transcriptions")]
#[case::audio_speech(29, "/audio/speech")]
#[case::v1_audio_speech(30, "/v1/audio/speech")]
#[case::moderations(31, "/moderations")]
#[case::v1_moderations(32, "/v1/moderations")]
#[case::v1_batches(33, "/v1/batches")]
#[case::batches(34, "/batches")]
#[case::v1_batches_item(35, "/v1/batches/{batch_id}")]
#[case::batches_item(36, "/batches/{batch_id}")]
#[case::v1_batches_item_cancel(37, "/v1/batches/{batch_id}/cancel")]
#[case::batches_item_cancel(38, "/batches/{batch_id}/cancel")]
#[case::v1_files(39, "/v1/files")]
#[case::files(40, "/files")]
#[case::v1_files_item(41, "/v1/files/{file_id}")]
#[case::files_item(42, "/files/{file_id}")]
#[case::v1_files_item_content(43, "/v1/files/{file_id}/content")]
#[case::files_item_content(44, "/files/{file_id}/content")]
#[case::tuning_jobs(45, "/fine_tuning/jobs")]
#[case::v1_tuning_jobs(46, "/v1/fine_tuning/jobs")]
#[case::tuning_jobs_item_cancel(47, "/fine_tuning/jobs/{fine_tuning_job_id}/cancel")]
#[case::v1_tuning_jobs_item_cancel(48, "/v1/fine_tuning/jobs/{fine_tuning_job_id}/cancel")]
#[case::assistants(49, "/assistants")]
#[case::v1_assistants(50, "/v1/assistants")]
#[case::v1_assistants_item(51, "/v1/assistants/{assistant_id}")]
#[case::assistants_item(52, "/assistants/{assistant_id}")]
#[case::threads(53, "/threads")]
#[case::v1_threads(54, "/v1/threads")]
#[case::threads_item(55, "/threads/{thread_id}")]
#[case::v1_threads_item(56, "/v1/threads/{thread_id}")]
#[case::threads_item_messages(57, "/threads/{thread_id}/messages")]
#[case::v1_threads_item_messages(58, "/v1/threads/{thread_id}/messages")]
#[case::threads_item_runs(59, "/threads/{thread_id}/runs")]
#[case::v1_threads_item_runs(60, "/v1/threads/{thread_id}/runs")]
#[case::models(61, "/models")]
#[case::v1_models(62, "/v1/models")]
#[case::utils_token_counter(63, "/utils/token_counter")]
#[case::utils_model_info(64, "/utils/model_info")]
#[case::utils_transform_request(65, "/utils/transform_request")]
#[case::rerank(66, "/rerank")]
#[case::v1_rerank(67, "/v1/rerank")]
#[case::v2_rerank(68, "/v2/rerank")]
#[case::realtime(69, "/realtime")]
#[case::v1_realtime(70, "/v1/realtime")]
#[case::openai_v1_realtime(71, "/openai/v1/realtime")]
#[case::realtime_query(72, "/realtime?{model}")]
#[case::v1_realtime_query(73, "/v1/realtime?{model}")]
#[case::openai_v1_realtime_query(74, "/openai/v1/realtime?{model}")]
#[case::realtime_secrets(75, "/realtime/client_secrets")]
#[case::v1_realtime_secrets(76, "/v1/realtime/client_secrets")]
#[case::openai_v1_realtime_secrets(77, "/openai/v1/realtime/client_secrets")]
#[case::realtime_calls(78, "/realtime/calls")]
#[case::v1_realtime_calls(79, "/v1/realtime/calls")]
#[case::openai_v1_realtime_calls(80, "/openai/v1/realtime/calls")]
#[case::realtime_sessions(81, "/realtime/transcription_sessions")]
#[case::v1_realtime_sessions(82, "/v1/realtime/transcription_sessions")]
#[case::openai_v1_realtime_sessions(83, "/openai/v1/realtime/transcription_sessions")]
#[case::responses(84, "/responses")]
#[case::v1_responses(85, "/v1/responses")]
#[case::openai_v1_responses(86, "/openai/v1/responses")]
#[case::responses_item(87, "/responses/{response_id}")]
#[case::v1_responses_item(88, "/v1/responses/{response_id}")]
#[case::openai_v1_responses_item(89, "/openai/v1/responses/{response_id}")]
#[case::responses_item_input_items(90, "/responses/{response_id}/input_items")]
#[case::v1_responses_item_input_items(91, "/v1/responses/{response_id}/input_items")]
#[case::openai_v1_responses_item_input_items(92, "/openai/v1/responses/{response_id}/input_items")]
#[case::responses_item_cancel(93, "/responses/{response_id}/cancel")]
#[case::v1_responses_item_cancel(94, "/v1/responses/{response_id}/cancel")]
#[case::openai_v1_responses_item_cancel(95, "/openai/v1/responses/{response_id}/cancel")]
#[case::responses_input_tokens(96, "/responses/input_tokens")]
#[case::v1_responses_input_tokens(97, "/v1/responses/input_tokens")]
#[case::openai_v1_responses_input_tokens(98, "/openai/v1/responses/input_tokens")]
#[case::stores(99, "/vector_stores")]
#[case::v1_stores(100, "/v1/vector_stores")]
#[case::stores_item(101, "/vector_stores/{vector_store_id}")]
#[case::v1_stores_item(102, "/v1/vector_stores/{vector_store_id}")]
#[case::stores_item_search(103, "/vector_stores/{vector_store_id}/search")]
#[case::v1_stores_item_search(104, "/v1/vector_stores/{vector_store_id}/search")]
#[case::stores_item_files(105, "/vector_stores/{vector_store_id}/files")]
#[case::v1_stores_item_files(106, "/v1/vector_stores/{vector_store_id}/files")]
#[case::stores_item_files_item(107, "/vector_stores/{vector_store_id}/files/{file_id}")]
#[case::v1_stores_item_files_item(108, "/v1/vector_stores/{vector_store_id}/files/{file_id}")]
#[case::stores_item_files_item_content(
    109,
    "/vector_stores/{vector_store_id}/files/{file_id}/content"
)]
#[case::v1_store_file_content(110, "/v1/vector_stores/{vector_store_id}/files/{file_id}/content")]
#[case::store_list(111, "/vector_store/list")]
#[case::v1_store_list(112, "/v1/vector_store/list")]
#[case::search(113, "/search")]
#[case::v1_search(114, "/v1/search")]
#[case::search_item(115, "/search/{search_tool_name}")]
#[case::v1_search_item(116, "/v1/search/{search_tool_name}")]
#[case::decisions(117, "/decisions")]
#[case::v1_decisions(118, "/v1/decisions")]
#[case::systemone(119, "/systemone")]
#[case::v1_systemone(120, "/v1/systemone")]
#[case::ocr(121, "/ocr")]
#[case::v1_ocr(122, "/v1/ocr")]
#[case::containers(123, "/containers")]
#[case::v1_containers(124, "/v1/containers")]
#[case::containers_wildcard(125, "/containers/*")]
#[case::v1_containers_wildcard(126, "/v1/containers/*")]
fn literal_membership(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(OPENAI_ROUTES.get(index), Some(&literal));
}

#[rstest]
#[case::v1_messages(0, "/v1/messages")]
#[case::v1_messages_count_tokens(1, "/v1/messages/count_tokens")]
#[case::gateway_v1_messages(2, "/claude_code_gateway/v1/messages")]
#[case::gateway_count_tokens(3, "/claude_code_gateway/v1/messages/count_tokens")]
#[case::v1_skills(4, "/v1/skills")]
#[case::v1_skills_item(5, "/v1/skills/{skill_id}")]
#[case::claude_code_marketplace_json(6, "/claude-code/marketplace.json")]
#[case::claude_code_plugins(7, "/claude-code/plugins")]
#[case::claude_code_plugins_item(8, "/claude-code/plugins/{plugin_name}")]
fn alias_membership(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(ANTHROPIC_ROUTES.get(index), Some(&literal));
}

#[rstest]
#[case::unknown_path("/chat")]
#[case::wrong_query_spelling("/realtime?model={model}")]
#[case::fine_tuning_retrieve("/fine_tuning/jobs/{fine_tuning_job_id}")]
#[case::fine_tuning_checkpoints("/fine_tuning/jobs/{fine_tuning_job_id}/checkpoints")]
#[case::plural_store_list("/vector_stores/list")]
#[case::anthropic_unversioned("/messages")]
#[case::expanded_model("/engines/gpt-4/chat/completions")]
#[case::expanded_wildcard("/containers/example")]
#[case::mcp_management("/v1/mcp/server")]
#[case::agent_management("/v1/agents")]
fn literal_nonmembership(#[case] literal: &str) {
    assert!(!llm_api_routes().contains(&literal));
}

#[test]
fn group_expansion() {
    let expected = [
        OPENAI_ROUTES,
        ANTHROPIC_ROUTES,
        auxiliary_routes::GOOGLE_ROUTES,
        auxiliary_routes::MAPPED_PASS_THROUGH_ROUTES,
        auxiliary_routes::PASSTHROUGH_ROUTES_WILDCARD,
        auxiliary_routes::APPLY_GUARDRAIL_ROUTES,
        auxiliary_routes::MCP_INFERENCE_ROUTES,
        auxiliary_routes::LITELLM_NATIVE_ROUTES,
        auxiliary_routes::AGENT_INFERENCE_ROUTES,
        auxiliary_routes::MODEL_INFO_ROUTES,
    ]
    .concat();
    let actual = llm_api_routes();
    assert_eq!(OPENAI_ROUTES.len(), 127);
    assert_eq!(ANTHROPIC_ROUTES.len(), 9);
    assert_eq!(actual.len(), 251);
    assert_eq!(actual, expected);
}
