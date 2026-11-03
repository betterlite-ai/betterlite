#![cfg(test)]

use rstest::rstest;

use super::canonical::{
    RequestRouteSource, ScopeFields, get_request_route, get_request_route_template,
};
use super::normalize::RouteNormalizer;
use super::public_routes::{RoutePatternError, check_configured_public_routes, is_public_route};

#[rstest]
#[case::scope_path_beats_host_url(Some("/api/chat"), None, None, "/wrong", "/api/chat")]
#[case::missing_scope_path_fallback(None, None, None, "/api/chat", "/api/chat")]
#[case::app_root_wins(Some("/api/chat"), Some("/api"), Some("/other"), "/url", "/chat")]
#[case::root_trailing_slashes(Some("/api/chat"), Some("/api///"), None, "/url", "/chat")]
#[case::root_exact_becomes_slash(Some("/api"), Some("/api"), None, "/url", "/")]
#[case::root_whole_segment(Some("/api/chat"), Some("/api"), None, "/url", "/chat")]
#[case::sibling_root_kept(Some("/apiary/chat"), Some("/api"), None, "/url", "/apiary/chat")]
#[case::root_slash_kept(Some("/api/chat"), Some("/"), None, "/url", "/api/chat")]
fn request_path(
    #[case] path: Option<&str>,
    #[case] app_root: Option<&str>,
    #[case] root: Option<&str>,
    #[case] url_path: &str,
    #[case] expected: &str,
) {
    let source = RequestRouteSource {
        scope: Some(ScopeFields {
            path,
            app_root_path: app_root,
            root_path: root,
        }),
        url_path,
    };
    assert_eq!(get_request_route(&source), expected);
}

#[test]
fn malformed_scope_fallback() {
    let source = RequestRouteSource {
        scope: None,
        url_path: "/url-path",
    };
    assert_eq!(get_request_route(&source), "/url-path");
}

#[rstest]
#[case::nonempty_route_template(Some("/v1/models/{model}"), Some("/v1/models/{model}"))]
#[case::absent_empty_nonstring_template(None, None)]
fn route_template(#[case] template: Option<&str>, #[case] expected: Option<&str>) {
    assert_eq!(get_request_route_template(template), expected);
}

#[rstest]
#[case::wildcard_empty_prefix("/anything", "*", true)]
#[case::wildcard_one_terminal_star("/api/thing", "/api/*", true)]
#[case::embedded_star_literal("/api/a*b", "/api/a*b", true)]
#[case::wildcard_case_sensitive("/API/x", "/api/*", false)]
fn wildcard_cases(#[case] path: &str, #[case] pattern: &str, #[case] expected: bool) {
    assert_eq!(is_public_route(path, &[pattern.to_owned()]), expected);
}

#[test]
fn exact_before_wildcard() {
    let routes = vec!["/api/exact".to_owned(), "/api/*".to_owned()];
    assert!(is_public_route("/api/exact", &routes));
}

#[test]
fn group_name_not_expanded() {
    assert!(!is_public_route(
        "/health",
        &["LiteLLMRoutes.public_routes".to_owned()]
    ));
}

#[test]
fn malformed_public_list_logs_false() {
    assert!(!check_configured_public_routes(
        "/api",
        Err(RoutePatternError::MalformedSettings)
    ));
}

#[test]
fn native_general_settings_public_routes_works_without_premium_user() {
    assert!(is_public_route("/api/users", &["/api/*".to_owned()]));
}

#[rstest]
#[case::normalize_responses_item("/v1/responses/abc", "/v1/responses/{response_id}")]
#[case::normalize_responses_input_items(
    "/v1/responses/abc/input_items",
    "/v1/responses/{response_id}/input_items"
)]
#[case::normalize_responses_cancel(
    "/openai/v1/responses/abc/cancel",
    "/openai/v1/responses/{response_id}/cancel"
)]
#[case::normalize_threads_item("/v1/threads/t1", "/v1/threads/{thread_id}")]
#[case::normalize_threads_runs("/v1/threads/t1/runs", "/v1/threads/{thread_id}/runs")]
#[case::normalize_threads_run("/v1/threads/t1/runs/r1", "/v1/threads/{thread_id}/runs/{run_id}")]
#[case::normalize_threads_steps(
    "/v1/threads/t1/runs/r1/steps",
    "/v1/threads/{thread_id}/runs/{run_id}/steps"
)]
#[case::normalize_threads_step(
    "/v1/threads/t1/runs/r1/steps/s1",
    "/v1/threads/{thread_id}/runs/{run_id}/steps/{step_id}"
)]
#[case::normalize_threads_cancel(
    "/v1/threads/t1/runs/r1/cancel",
    "/v1/threads/{thread_id}/runs/{run_id}/cancel"
)]
#[case::normalize_threads_submit_tool_outputs(
    "/v1/threads/t1/runs/r1/submit_tool_outputs",
    "/v1/threads/{thread_id}/runs/{run_id}/submit_tool_outputs"
)]
#[case::normalize_threads_messages("/v1/threads/t1/messages", "/v1/threads/{thread_id}/messages")]
#[case::normalize_threads_message(
    "/v1/threads/t1/messages/m1",
    "/v1/threads/{thread_id}/messages/{message_id}"
)]
#[case::normalize_vector_stores_item("/v1/vector_stores/v1", "/v1/vector_stores/{vector_store_id}")]
#[case::normalize_vector_stores_files(
    "/v1/vector_stores/v1/files",
    "/v1/vector_stores/{vector_store_id}/files"
)]
#[case::normalize_vector_stores_file(
    "/v1/vector_stores/v1/files/f1",
    "/v1/vector_stores/{vector_store_id}/files/{file_id}"
)]
#[case::normalize_vector_stores_file_batches(
    "/v1/vector_stores/v1/file_batches",
    "/v1/vector_stores/{vector_store_id}/file_batches"
)]
#[case::normalize_vector_stores_batch(
    "/v1/vector_stores/v1/file_batches/b1",
    "/v1/vector_stores/{vector_store_id}/file_batches/{batch_id}"
)]
#[case::normalize_assistants_item("/v1/assistants/a1", "/v1/assistants/{assistant_id}")]
#[case::normalize_files_item("/openai/v1/files/f1", "/openai/v1/files/{file_id}")]
#[case::normalize_files_content("/v1/files/f1/content", "/v1/files/{file_id}/content")]
#[case::normalize_batches_item("/v1/batches/b1", "/v1/batches/{batch_id}")]
#[case::normalize_batches_cancel("/v1/batches/b1/cancel", "/v1/batches/{batch_id}/cancel")]
#[case::normalize_fine_tuning_jobs_item(
    "/v1/fine_tuning/jobs/j1",
    "/v1/fine_tuning/jobs/{fine_tuning_job_id}"
)]
#[case::normalize_fine_tuning_jobs_events(
    "/v1/fine_tuning/jobs/j1/events",
    "/v1/fine_tuning/jobs/{fine_tuning_job_id}/events"
)]
#[case::normalize_fine_tuning_jobs_cancel(
    "/v1/fine_tuning/jobs/j1/cancel",
    "/v1/fine_tuning/jobs/{fine_tuning_job_id}/cancel"
)]
#[case::normalize_fine_tuning_jobs_checkpoints(
    "/v1/fine_tuning/jobs/j1/checkpoints",
    "/v1/fine_tuning/jobs/{fine_tuning_job_id}/checkpoints"
)]
#[case::normalize_models_item("/v1/models/gpt-x", "/v1/models/{model}")]
#[case::openai_v1_threads_item("/openai/v1/threads/t1", "/openai/v1/threads/{thread_id}")]
#[case::unversioned_responses("/responses/r1", "/responses/{response_id}")]
#[case::unversioned_others_unchanged("/files/f1", "/files/f1")]
#[case::trailing_slash_unchanged("/v1/responses/r1/", "/v1/responses/r1/")]
#[case::unknown_suffix_unchanged("/v1/responses/r1/other", "/v1/responses/r1/other")]
#[case::empty_id_unchanged("/v1/responses/", "/v1/responses/")]
#[case::already_templated_path("/v1/responses/{response_id}", "/v1/responses/{response_id}")]
fn normalization(#[case] path: &str, #[case] expected: &str) {
    let normalizer = RouteNormalizer::default();
    assert_eq!(normalizer.normalize_request_route(path), expected);
}

#[test]
fn normalization_cache_bound() {
    let normalizer = RouteNormalizer::default();
    (0_u32..300_u32)
        .map(|index| format!("/unmatched/{index}"))
        .for_each(|path| {
            assert_eq!(normalizer.normalize_request_route(&path), path);
        });
    assert_eq!(
        normalizer.normalize_request_route("/v1/models/m"),
        "/v1/models/{model}"
    );
}
