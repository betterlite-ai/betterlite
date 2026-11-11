use rstest::rstest;

use crate::datadog_llm_observability::error::DatadogLlmObsError;
use crate::datadog_llm_observability::settings::DatadogLlmObsSettings;
use crate::datadog_llm_observability::tests::support;

#[derive(Clone, Copy)]
enum Expected<'expected> {
    Url { url: &'expected str, key: bool },
    MissingApiKey,
    MissingSite,
}

fn assert_endpoint(
    outcome: Result<DatadogLlmObsSettings, DatadogLlmObsError>,
    expected: Expected<'_>,
) {
    match expected {
        Expected::Url { url, key } => match outcome {
            Ok(settings) => {
                assert_eq!(settings.intake_url, url);
                assert_eq!(settings.api_key.is_some(), key);
            }
            Err(error) => panic!("expected {url}, got {error}"),
        },
        Expected::MissingApiKey => {
            assert!(matches!(outcome, Err(DatadogLlmObsError::MissingApiKey)));
        }
        Expected::MissingSite => {
            assert!(matches!(outcome, Err(DatadogLlmObsError::MissingSite)));
        }
    }
}

#[rstest]
#[case::direct_from_environment(
    &[("DD_API_KEY", "key-1"), ("DD_SITE", "us5.datadoghq.com")],
    Expected::Url { url: "https://api.us5.datadoghq.com/api/intake/llm-obs/v1/trace/spans", key: true }
)]
#[case::agent_from_environment(
    &[("LITELLM_DD_AGENT_HOST", "agent.local")],
    Expected::Url { url: "http://agent.local:8126/api/intake/llm-obs/v1/trace/spans", key: false }
)]
#[case::agent_port_from_environment(
    &[("LITELLM_DD_AGENT_HOST", "agent.local"), ("LITELLM_DD_LLM_OBS_PORT", "9000")],
    Expected::Url { url: "http://agent.local:9000/api/intake/llm-obs/v1/trace/spans", key: false }
)]
#[case::agent_keeps_a_configured_key(
    &[("LITELLM_DD_AGENT_HOST", "agent.local"), ("DD_API_KEY", "key-1")],
    Expected::Url { url: "http://agent.local:8126/api/intake/llm-obs/v1/trace/spans", key: true }
)]
#[case::base_url_overrides_agent_mode(
    &[("LITELLM_DD_AGENT_HOST", "agent.local"), ("DD_BASE_URL", "http://localhost:9000")],
    Expected::Url { url: "http://localhost:9000/api/intake/llm-obs/v1/trace/spans", key: false }
)]
#[case::base_url_overrides_direct_mode(
    &[("DD_API_KEY", "key-1"), ("DD_SITE", "us5.datadoghq.com"), ("DD_BASE_URL", "http://localhost:9000")],
    Expected::Url { url: "http://localhost:9000/api/intake/llm-obs/v1/trace/spans", key: true }
)]
#[case::base_url_keeps_a_trailing_slash(
    &[("DD_BASE_URL", "http://localhost:9000/"), ("DD_API_KEY", "key-1"), ("DD_SITE", "us5.datadoghq.com")],
    Expected::Url { url: "http://localhost:9000//api/intake/llm-obs/v1/trace/spans", key: true }
)]
#[case::empty_base_url_is_unset(
    &[("DD_BASE_URL", ""), ("DD_API_KEY", "key-1"), ("DD_SITE", "us5.datadoghq.com")],
    Expected::Url { url: "https://api.us5.datadoghq.com/api/intake/llm-obs/v1/trace/spans", key: true }
)]
#[case::empty_agent_host_falls_back_to_direct(
    &[("LITELLM_DD_AGENT_HOST", ""), ("DD_API_KEY", "key-1"), ("DD_SITE", "us5.datadoghq.com")],
    Expected::Url { url: "https://api.us5.datadoghq.com/api/intake/llm-obs/v1/trace/spans", key: true }
)]
#[case::empty_agent_port_takes_the_default(
    &[("LITELLM_DD_AGENT_HOST", "agent.local"), ("LITELLM_DD_LLM_OBS_PORT", "")],
    Expected::Url { url: "http://agent.local:8126/api/intake/llm-obs/v1/trace/spans", key: false }
)]
#[case::missing_api_key(&[("DD_SITE", "us5.datadoghq.com")], Expected::MissingApiKey)]
#[case::empty_api_key_is_missing(&[("DD_API_KEY", ""), ("DD_SITE", "us5.datadoghq.com")], Expected::MissingApiKey)]
#[case::missing_site(&[("DD_API_KEY", "key-1")], Expected::MissingSite)]
#[case::empty_site_is_missing(&[("DD_API_KEY", "key-1"), ("DD_SITE", "")], Expected::MissingSite)]
fn datadog_llm_obs_endpoint_configuration_behavior(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Expected<'_>,
) {
    let init = support::init(None, None, None, None);
    let outcome = support::resolve(&init, &support::environment(pairs));
    assert_endpoint(outcome, expected);
}

#[test]
fn datadog_llm_obs_explicit_credentials_override_the_environment() {
    let init = support::init(Some("explicit-key"), Some("explicit.site"), None, None);
    let environment = support::environment(&[("DD_API_KEY", "env-key"), ("DD_SITE", "env.site")]);
    let settings = support::resolve(&init, &environment).expect("the endpoint resolves");
    assert_eq!(
        settings.intake_url,
        "https://api.explicit.site/api/intake/llm-obs/v1/trace/spans"
    );
    assert!(settings.api_key.is_some());
}

#[test]
fn datadog_llm_obs_explicit_agent_endpoint_overrides_the_environment() {
    let init = support::init(None, None, Some("explicit.local"), Some("9000"));
    let environment = support::environment(&[
        ("LITELLM_DD_AGENT_HOST", "env-agent.local"),
        ("LITELLM_DD_LLM_OBS_PORT", "8127"),
    ]);
    let settings = support::resolve(&init, &environment).expect("the endpoint resolves");
    assert_eq!(
        settings.intake_url,
        "http://explicit.local:9000/api/intake/llm-obs/v1/trace/spans"
    );
}
