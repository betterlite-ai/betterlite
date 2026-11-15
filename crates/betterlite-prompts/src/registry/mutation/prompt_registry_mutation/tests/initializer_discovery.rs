#![cfg(test)]

use rstest::rstest;

use super::initializers::{
    PROMPT_INITIALIZER_REGISTRY, discover_prompt_initializers, prompt_initializer,
};
use super::support::prompt_spec;

#[test]
fn discovers_the_four_supported_integrations() {
    let discovered = discover_prompt_initializers();
    assert_eq!(
        discovered.keys().copied().collect::<Vec<_>>(),
        [
            "dotprompt",
            "bitbucket",
            "gitlab",
            "generic_prompt_management",
        ]
    );
}

#[test]
fn builds_the_map_from_the_compile_time_registry() {
    let discovered = discover_prompt_initializers();
    assert_eq!(discovered.len(), PROMPT_INITIALIZER_REGISTRY.len());
    assert!(
        PROMPT_INITIALIZER_REGISTRY
            .iter()
            .all(|(name, initializer)| discovered.get(name).copied() == Some(*initializer))
    );
}

#[rstest]
#[case::langfuse("langfuse")]
#[case::custom("custom")]
#[case::arize_phoenix("arize_phoenix")]
#[case::unknown("some_new_backend")]
fn discovers_no_initializer_for_an_integration_without_one(#[case] integration: &str) {
    assert!(discover_prompt_initializers().get(integration).is_none());
    assert!(prompt_initializer(integration).is_none());
}

#[rstest]
#[case::dotprompt_strips_the_version("dotprompt", None, "greeting.v3", Some("greeting"))]
#[case::dotprompt_keeps_the_params_id("dotprompt", Some("chat/hi"), "greeting.v3", Some("chat/hi"))]
#[case::dotprompt_keeps_an_unversioned_id("dotprompt", None, "greeting", Some("greeting"))]
#[case::dotprompt_keeps_an_empty_params_id("dotprompt", Some(""), "greeting.v3", Some(""))]
#[case::gitlab_uses_the_spec_id("gitlab", Some("ignored"), "chat/hi", Some("chat/hi"))]
#[case::bitbucket_uses_the_params_id("bitbucket", Some("bb-id"), "greeting.v3", Some("bb-id"))]
#[case::bitbucket_without_a_params_id("bitbucket", None, "greeting.v3", None)]
#[case::generic_uses_the_params_id(
    "generic_prompt_management",
    Some("gen"),
    "greeting.v3",
    Some("gen")
)]
#[case::generic_without_a_params_id("generic_prompt_management", None, "greeting.v3", None)]
fn each_initializer_builds_the_client_its_backend_fetches(
    #[case] integration: &str,
    #[case] params_prompt_id: Option<&str>,
    #[case] spec_prompt_id: &str,
    #[case] expected_prompt_id: Option<&str>,
) {
    let mut spec = prompt_spec(spec_prompt_id, "development", integration);
    spec.litellm_params.prompt_id = params_prompt_id.map(str::to_owned);
    let backend = prompt_initializer(integration).expect("the integration has an initializer");
    let callback = backend.initializer(&spec.litellm_params, &spec);
    assert_eq!(callback.prompt_id.as_deref(), expected_prompt_id);
}

#[test]
fn a_registered_client_holds_no_compiled_template_yet() {
    let spec = prompt_spec("greeting.v3", "development", "dotprompt");
    let backend = prompt_initializer("dotprompt").expect("dotprompt has an initializer");
    let callback = backend.initializer(&spec.litellm_params, &spec);
    assert!(callback.prompt_template.is_empty());
    assert_eq!(callback.prompt_template_model, None);
    assert_eq!(callback.prompt_optional_params, None);
    assert_eq!(callback.completed_messages, None);
}
