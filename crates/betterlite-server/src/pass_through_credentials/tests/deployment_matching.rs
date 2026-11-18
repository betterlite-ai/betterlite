#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{Json, PassThroughDeployment, available, deployment, obj, router, text, vertex};

fn openai(api_key: &str) -> PassThroughDeployment {
    available("gpt", "openai/gpt-4o-mini", &[("api_key", text(api_key))])
}

fn assemblyai(api_base: &str, api_key: &str) -> PassThroughDeployment {
    available(
        "transcribe",
        "assemblyai/universal-2",
        &[("api_base", text(api_base)), ("api_key", text(api_key))],
    )
}

#[test]
fn another_providers_deployment_lends_no_key() {
    let resolver = router(vec![openai("openai-key")], &[]);
    assert!(resolver.get_credentials("anthropic", None).is_none());
    let openai_key = resolver
        .get_credentials("openai", None)
        .expect("openai key");
    assert_eq!(openai_key.expose_secret(), "openai-key");
}

#[test]
fn the_region_must_match_the_deployments_api_base() {
    let resolver = router(
        vec![
            assemblyai("https://api.eu.assemblyai.com", "eu-key"),
            assemblyai("https://api.assemblyai.com", "us-key"),
        ],
        &[],
    );
    let eu = resolver
        .get_credentials("assemblyai", Some("eu"))
        .expect("eu key");
    assert_eq!(eu.expose_secret(), "eu-key");
    let unset = resolver
        .get_credentials("assemblyai", None)
        .expect("us key");
    assert_eq!(unset.expose_secret(), "us-key");
    assert!(
        resolver
            .get_credentials("assemblyai", Some("us-west"))
            .is_none()
    );
}

#[test]
fn only_assemblyai_reads_a_region_from_the_api_base() {
    assert_eq!(
        super::super::region_name_from_api_base(
            "assemblyai",
            Some("https://api.eu.assemblyai.com")
        ),
        Some("eu")
    );
    assert_eq!(
        super::super::region_name_from_api_base("assemblyai", Some("https://api.assemblyai.com")),
        None
    );
    assert_eq!(
        super::super::region_name_from_api_base("openai", Some("https://api.eu.openai.com")),
        None
    );
    assert_eq!(
        super::super::region_name_from_api_base("assemblyai", None),
        None
    );
}

#[test]
fn the_first_matching_deployment_wins() {
    let resolver = router(vec![openai("first-key"), openai("second-key")], &[]);
    let api_key = resolver.get_credentials("openai", None).expect("first key");
    assert_eq!(api_key.expose_secret(), "first-key");
}

#[rstest]
#[case::deployment_name("public-gemini", Some("project-one"))]
#[case::upstream_model("vertex_ai/publishers/google/gemini-2.0", Some("project-one"))]
#[case::suffix("publishers/google/gemini-2.0", Some("project-one"))]
#[case::last_component("gemini-2.0", None)]
fn model_names_match_the_name_and_the_suffix_after_the_first_slash(
    #[case] named: &str,
    #[case] expected: Option<&str>,
) {
    let published = available(
        "public-gemini",
        "vertex_ai/publishers/google/gemini-2.0",
        &[
            ("vertex_project", text("project-one")),
            ("vertex_location", text("us-central1")),
        ],
    );
    let other = vertex("other", "project-two", "us-central1");
    let resolver = router(vec![published, other], &[]);
    let project = resolver
        .vertex_deployment_credentials(Some(named))
        .and_then(|record| record.vertex_project);
    assert_eq!(project.as_deref(), expected);
}

#[test]
fn provider_errors_missing_models_and_explicit_providers() {
    let mut resolver = router(
        vec![
            available("unknown", "unknown", &[("api_key", text("wrong"))]),
            deployment("missing", obj(&[("use_in_pass_through", Json::Bool(true))])),
        ],
        &[],
    );
    assert!(resolver.get_credentials("openai", None).is_none());
    resolver.deployments.push(available(
        "explicit",
        "other/model",
        &[
            ("custom_llm_provider", text("openai")),
            ("api_key", text("declared")),
        ],
    ));
    assert_eq!(
        resolver
            .get_credentials("openai", None)
            .unwrap()
            .expose_secret(),
        "declared"
    );
}
