#![cfg(test)]

use super::{AnalysisBody, RequestIds, SYSTEM};

const fn sample(team_id: Option<&str>) -> AnalysisBody<'_> {
    AnalysisBody::new(
        "lens-model",
        "hello",
        RequestIds {
            lens_id: "lens-1",
            run_id: "job-1",
            worker_id: "worker-1",
            team_id,
        },
    )
}

#[test]
fn the_body_carries_the_chat_parameters_litellm_sends() {
    let json = serde_json::to_string(&sample(Some("team-1"))).unwrap();
    let expected = [
        "{\"model\":\"lens-model\",\"messages\":[{\"role\":\"system\",\"content\":\"",
        SYSTEM,
        "\"},{\"role\":\"user\",\"content\":\"hello\"}],",
        "\"max_tokens\":4096,\"stream\":false,\"timeout\":120,\"num_retries\":0,",
        "\"disable_fallbacks\":true,\"response_format\":{\"type\":\"json_object\"},",
        "\"metadata\":{\"tags\":[\"litellm-lens\"],\"lens_id\":\"lens-1\",",
        "\"lens_run_id\":\"job-1\",\"lens_worker_id\":\"worker-1\",",
        "\"user_api_key_team_id\":\"team-1\"}}",
    ]
    .concat();
    assert_eq!(json, expected);
}

#[test]
fn the_system_instruction_is_the_litellm_text() {
    assert_eq!(SYSTEM.len(), 362);
}

#[test]
fn a_missing_team_is_sent_as_null() {
    let json = serde_json::to_string(&sample(None)).unwrap();
    assert!(json.ends_with("\"user_api_key_team_id\":null}}"));
}
