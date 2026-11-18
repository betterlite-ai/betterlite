#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{DeploymentParameters, Json, PassThroughDeployment, available, router, text};

fn openai(flag: bool, api_key: &str) -> PassThroughDeployment {
    available(
        "gpt",
        "openai/gpt-4o-mini",
        &[
            ("api_key", text(api_key)),
            ("use_in_pass_through", Json::Bool(flag)),
        ],
    )
}

#[test]
fn a_matching_deployment_key_wins_over_the_environment() {
    let resolver = router(
        vec![openai(true, "deployment-key")],
        &[("OPENAI_API_KEY", "environment-key")],
    );
    let api_key = resolver
        .get_credentials("openai", None)
        .expect("deployment key");
    assert_eq!(api_key.expose_secret(), "deployment-key");
}

#[test]
fn an_unflagged_deployment_leaves_the_environment_to_answer() {
    let resolver = router(
        vec![openai(false, "deployment-key")],
        &[("OPENAI_API_KEY", "environment-key")],
    );
    let api_key = resolver
        .get_credentials("openai", None)
        .expect("environment key");
    assert_eq!(api_key.expose_secret(), "environment-key");
}

#[test]
fn nothing_configured_resolves_to_none() {
    let unflagged = router(vec![openai(false, "deployment-key")], &[]);
    assert!(unflagged.get_credentials("openai", None).is_none());
    let empty = router(vec![], &[]);
    assert!(empty.get_credentials("openai", None).is_none());
    let blank = router(vec![], &[("OPENAI_API_KEY", "")]);
    assert!(blank.get_credentials("openai", None).is_none());
}
#[test]
fn missing_keys_skip_only_matching_deployments() {
    let mut resolver = router(
        vec![
            available("missing", "openai/gpt", &[]),
            available(
                "unrelated",
                "anthropic/claude",
                &[("api_key", text("wrong"))],
            ),
        ],
        &[("OPENAI_API_KEY", "fallback")],
    );
    assert_eq!(
        resolver
            .get_credentials("openai", None)
            .unwrap()
            .expose_secret(),
        "fallback"
    );
    resolver.deployments.push(openai(true, ""));
    assert_eq!(
        resolver
            .get_credentials("openai", None)
            .unwrap()
            .expose_secret(),
        ""
    );
}
#[rstest]
#[case::absent("{}", false)]
#[case::null(r#"{"use_in_pass_through":null}"#, false)]
#[case::disabled(r#"{"use_in_pass_through":false}"#, false)]
#[case::enabled(r#"{"use_in_pass_through":true}"#, true)]
fn pass_through_flag_defaults_only_when_absent_or_null(
    #[case] input: &str,
    #[case] expected: bool,
) {
    let params: DeploymentParameters = serde_json::from_str(input).unwrap();
    assert_eq!(params.use_in_pass_through, expected);
}

#[rstest]
#[case::text(r#"{"use_in_pass_through":"true"}"#)]
#[case::number(r#"{"use_in_pass_through":1}"#)]
#[case::list(r#"{"use_in_pass_through":[]}"#)]
fn pass_through_flag_rejects_non_boolean_values(#[case] input: &str) {
    assert!(serde_json::from_str::<DeploymentParameters>(input).is_err());
}
