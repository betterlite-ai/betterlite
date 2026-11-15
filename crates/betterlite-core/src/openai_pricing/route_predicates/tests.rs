#![cfg(test)]

use rstest::rstest;

use super::{
    is_openai_chat_completions_route, is_openai_embeddings_route, is_openai_image_editing_route,
    is_openai_image_generation_route, is_openai_responses_route,
};

#[rstest]
#[case::plain("https://api.openai.com/v1/chat/completions", true)]
#[case::subdomain("https://eu.api.openai.com/v1/chat/completions", true)]
#[case::suffix_path("https://api.openai.com/foo/v1/chat/completions/bar", true)]
#[case::azure_host("https://resource.openai.azure.com/v1/chat/completions", true)]
#[case::scheme_relative("//api.openai.com/v1/chat/completions", true)]
#[case::query_does_not_hide_path("https://api.openai.com/v1/chat/completions?api-version=1", true)]
#[case::query_only_match("https://api.openai.com/v1/models?next=/v1/chat/completions", false)]
#[case::singular("https://api.openai.com/v1/chat/completion", false)]
#[case::lookalike_glued("https://xapi.openai.com/v1/chat/completions", false)]
#[case::lookalike_suffix("https://api.openai.com.attacker.example/v1/chat/completions", false)]
#[case::other_host("https://evil.example/v1/chat/completions", false)]
#[case::relative("api.openai.com/v1/chat/completions", false)]
#[case::empty("", false)]
#[case::dot_segment("https://api.openai.com/v1/chat/completions/..", false)]
fn chat_completions(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_openai_chat_completions_route(route), expected);
}

#[rstest]
#[case::plain("https://api.openai.com/v1/images/generations", true)]
#[case::suffix_path("https://api.openai.com/v1/images/generations?x=1", true)]
#[case::singular("https://api.openai.com/v1/images/generation", false)]
#[case::edits_are_not_generations("https://api.openai.com/v1/images/edits", false)]
#[case::other_host("https://evil.example/v1/images/generations", false)]
#[case::lookalike_suffix("https://api.openai.com.evil/v1/images/generations", false)]
#[case::dot_segment("https://api.openai.com/v1/images/generations/..", false)]
fn image_generation(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_openai_image_generation_route(route), expected);
}

#[rstest]
#[case::plain("https://api.openai.com/v1/images/edits", true)]
#[case::azure_host("https://resource.cognitiveservices.azure.com/v1/images/edits", true)]
#[case::singular("https://api.openai.com/v1/images/edit", false)]
#[case::generations_are_not_edits("https://api.openai.com/v1/images/generations", false)]
#[case::other_host("https://evil.example/v1/images/edits", false)]
#[case::dot_segment("https://api.openai.com/v1/images/edits/..", false)]
fn image_editing(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_openai_image_editing_route(route), expected);
}

#[rstest]
#[case::v1("https://api.openai.com/v1/responses", true)]
#[case::bare("https://api.openai.com/responses", true)]
#[case::suffix_path("https://api.openai.com/foo/responses/bar", true)]
#[case::azure_without_marker("https://resource.openai.azure.com/responses", true)]
#[case::singular("https://api.openai.com/v1/response", false)]
#[case::bare_singular("https://api.openai.com/response", false)]
#[case::other_host("https://evil.example/v1/responses", false)]
#[case::dot_segment("https://api.openai.com/v1/responses/%2e%2e", false)]
fn responses(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_openai_responses_route(route), expected);
}

#[rstest]
#[case::plain("https://api.openai.com/v1/embeddings", true)]
#[case::subdomain("https://eu.api.openai.com/v1/embeddings", true)]
#[case::singular("https://api.openai.com/v1/embedding", false)]
#[case::other_host("https://evil.example/v1/embeddings", false)]
#[case::lookalike_glued("https://xapi.openai.com/v1/embeddings", false)]
#[case::dot_segment("https://api.openai.com/v1/embeddings/../", false)]
fn embeddings(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_openai_embeddings_route(route), expected);
}
