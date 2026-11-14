#![cfg(test)]

use rstest::rstest;

use crate::api::agents::{AgentsProvider, get_provider_agents_api_config};

#[test]
fn gemini_selects_the_gemini_agents_config() {
    assert_eq!(
        get_provider_agents_api_config(Some("gemini")),
        Some(AgentsProvider::Gemini)
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[case::capitalized(Some("Gemini"))]
#[case::openai(Some("openai"))]
#[case::vertex_ai(Some("vertex_ai"))]
fn other_providers_have_no_native_agents_api(#[case] provider: Option<&str>) {
    assert_eq!(get_provider_agents_api_config(provider), None);
}
