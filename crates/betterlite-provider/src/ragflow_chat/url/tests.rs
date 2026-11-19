#![cfg(test)]

use rstest::rstest;

use super::complete_url;
use crate::ragflow_chat::error::RagflowError;
use crate::ragflow_chat::tests::support::{Credentials, environment};

fn url(case: &Credentials) -> Result<String, RagflowError> {
    complete_url(&case.input())
}

fn base_case(model: &str, base: &str) -> Credentials {
    let mut case = Credentials::new(model);
    case.api_base = Some(String::from(base));
    case
}

#[rstest]
#[case::chat("ragflow/chat/abc/gpt-4o", "chats_openai")]
#[case::agent("ragflow/agent/abc/gpt-4o", "agents_openai")]
fn chat_and_agent_models_pick_their_paths(#[case] model: &str, #[case] path: &str) {
    let case = base_case(model, "http://host:9380");
    assert_eq!(
        url(&case).expect("the fixture url builds"),
        format!("http://host:9380/api/v1/{path}/abc/chat/completions")
    );
}

#[rstest]
#[case::one_slash("http://host:9380/")]
#[case::many_slashes("http://host:9380///")]
fn trailing_slashes_are_removed(#[case] base: &str) {
    let case = base_case("ragflow/chat/abc/m", base);
    assert_eq!(
        url(&case).expect("the fixture url builds"),
        "http://host:9380/api/v1/chats_openai/abc/chat/completions"
    );
}

#[rstest]
#[case::bare_api_v1("http://host:9380/api/v1")]
#[case::api_v1_with_slash("http://host:9380/api/v1/")]
#[case::bare_v1("http://host:9380/v1")]
#[case::v1_with_slash("http://host:9380/v1/")]
fn one_version_suffix_is_removed(#[case] base: &str) {
    let case = base_case("ragflow/chat/abc/m", base);
    assert_eq!(
        url(&case).expect("the fixture url builds"),
        "http://host:9380/api/v1/chats_openai/abc/chat/completions"
    );
}

#[rstest]
#[case::api_v1_inside_another_suffix("http://host:9380/api/v1/api/v1", "http://host:9380/api/v1")]
#[case::path_that_ends_with_v1("http://host:9380/apiv1", "http://host:9380/apiv1")]
#[case::v1_mid_path("http://host:9380/v1/root", "http://host:9380/v1/root")]
fn only_a_trailing_version_suffix_is_removed(#[case] base: &str, #[case] kept: &str) {
    let case = base_case("ragflow/chat/abc/m", base);
    assert_eq!(
        url(&case).expect("the fixture url builds"),
        format!("{kept}/api/v1/chats_openai/abc/chat/completions")
    );
}

#[rstest]
#[case::plain("abc", "abc")]
#[case::space("a b", "a%20b")]
#[case::query_and_fragment("a?b#c", "a%3Fb%23c")]
#[case::unreserved("my.id-1_2~3", "my.id-1_2~3")]
#[case::percent("%41", "%2541")]
#[case::unicode("\u{fc}", "%C3%BC")]
fn the_entity_id_is_percent_encoded(#[case] id: &str, #[case] encoded: &str) {
    let case = base_case(&format!("ragflow/chat/{id}/m"), "http://host:9380");
    assert_eq!(
        url(&case).expect("the fixture url builds"),
        format!("http://host:9380/api/v1/chats_openai/{encoded}/chat/completions")
    );
}

#[test]
fn an_empty_entity_id_is_refused() {
    let case = base_case("ragflow/chat//m", "http://host:9380");
    let error = url(&case).expect_err("an empty id is refused");
    assert_eq!(error.to_string(), "entity_id is required");
}

#[rstest]
#[case::dot(".")]
#[case::dot_dot("..")]
fn a_dot_entity_id_is_refused(#[case] id: &str) {
    let case = base_case(&format!("ragflow/chat/{id}/m"), "http://host:9380");
    let error = url(&case).expect_err("a dot id is refused");
    assert_eq!(error.to_string(), "entity_id cannot be a dot path segment");
}

#[test]
fn the_base_falls_through_empty_and_global_values() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.environment = environment(&[("RAGFLOW_API_BASE", "http://env:9380")]);
    case.api_base = Some(String::new());
    case.global_api_base = Some(String::from("http://global:9380"));
    assert_eq!(
        url(&case).expect("the fixture url builds"),
        "http://global:9380/api/v1/chats_openai/abc/chat/completions"
    );
    case.global_api_base = None;
    assert_eq!(
        url(&case).expect("the fixture url builds"),
        "http://env:9380/api/v1/chats_openai/abc/chat/completions"
    );
    case.api_base = Some(String::from("http://explicit:9380"));
    assert_eq!(
        url(&case).expect("the fixture url builds"),
        "http://explicit:9380/api/v1/chats_openai/abc/chat/completions"
    );
}

#[test]
fn a_missing_base_is_reported_before_the_model_is_parsed() {
    let case = Credentials::new("ragflow/chat");
    let error = url(&case).expect_err("a missing base is refused");
    assert!(matches!(error, RagflowError::ApiBaseRequired));
    assert_eq!(
        error.to_string(),
        "api_base is required for RAGFlow provider. Set it via api_base parameter, RAGFLOW_API_BASE environment variable, or litellm.api_base"
    );
}

#[test]
fn an_empty_base_setting_counts_as_unset() {
    let mut case = Credentials::new("ragflow/chat/abc/m");
    case.environment = environment(&[("RAGFLOW_API_BASE", "")]);
    let error = url(&case).expect_err("an empty base is unset");
    assert!(matches!(error, RagflowError::ApiBaseRequired));
}

#[test]
fn a_bad_model_is_reported_when_the_base_resolves() {
    let case = base_case("ragflow/completion/abc/m", "http://host:9380");
    let error = url(&case).expect_err("an unknown endpoint is refused");
    assert_eq!(
        error.to_string(),
        "Invalid RAGFlow endpoint type: completion. Must be 'chat' or 'agent'"
    );
}
