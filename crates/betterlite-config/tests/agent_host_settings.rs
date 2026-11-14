#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::agent_host_settings::Settings;
use rstest::rstest;
use secrecy::ExposeSecret as _;

#[derive(Debug, Clone, Copy)]
enum Host {
    Gradient,
    Heroku,
    Hyperbolic,
    Lambda,
    Inception,
}

fn load(environment: &[(&str, &str)]) -> Settings {
    load_with("", environment)
}

fn load_with(source: &str, environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .agent_host_settings
}

fn values(host: Host, settings: &Settings) -> (Option<String>, Option<String>) {
    match host {
        Host::Gradient => (
            settings.gradient_ai_agent_endpoint.clone(),
            settings
                .gradient_ai_api_key
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
        ),
        Host::Heroku => (
            settings.heroku_api_base.clone(),
            settings
                .heroku_api_key
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
        ),
        Host::Hyperbolic => (
            settings.hyperbolic_api_base.clone(),
            settings
                .hyperbolic_api_key
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
        ),
        Host::Lambda => (
            settings.lambda_api_base.clone(),
            settings
                .lambda_api_key
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
        ),
        Host::Inception => (
            settings.inception_api_base.clone(),
            settings
                .inception_api_key
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
        ),
    }
}

#[rstest]
#[case::gradient_absent(Host::Gradient, &[], None, None)]
#[case::gradient_supplied(
    Host::Gradient,
    &[
        ("GRADIENT_AI_AGENT_ENDPOINT", "https://agent.example"),
        ("GRADIENT_AI_API_KEY", "gradient-key"),
    ],
    Some("https://agent.example"),
    Some("gradient-key")
)]
#[case::gradient_empty_values_stay_present(
    Host::Gradient,
    &[("GRADIENT_AI_AGENT_ENDPOINT", ""), ("GRADIENT_AI_API_KEY", "")],
    Some(""),
    Some("")
)]
#[case::heroku_absent(Host::Heroku, &[], None, None)]
#[case::heroku_supplied(
    Host::Heroku,
    &[("HEROKU_API_BASE", "https://heroku.example"), ("HEROKU_API_KEY", "heroku-key")],
    Some("https://heroku.example"),
    Some("heroku-key")
)]
#[case::heroku_empty_values_stay_present(
    Host::Heroku,
    &[("HEROKU_API_BASE", ""), ("HEROKU_API_KEY", "")],
    Some(""),
    Some("")
)]
#[case::hyperbolic_absent(Host::Hyperbolic, &[], None, None)]
#[case::hyperbolic_supplied(
    Host::Hyperbolic,
    &[
        ("HYPERBOLIC_API_BASE", "https://hyperbolic.example/v1"),
        ("HYPERBOLIC_API_KEY", "hyperbolic-key"),
    ],
    Some("https://hyperbolic.example/v1"),
    Some("hyperbolic-key")
)]
#[case::hyperbolic_empty_values_stay_present(
    Host::Hyperbolic,
    &[("HYPERBOLIC_API_BASE", ""), ("HYPERBOLIC_API_KEY", "")],
    Some(""),
    Some("")
)]
#[case::lambda_absent(Host::Lambda, &[], None, None)]
#[case::lambda_supplied(
    Host::Lambda,
    &[("LAMBDA_API_BASE", "https://lambda.example/v1"), ("LAMBDA_API_KEY", "lambda-key")],
    Some("https://lambda.example/v1"),
    Some("lambda-key")
)]
#[case::lambda_empty_values_stay_present(
    Host::Lambda,
    &[("LAMBDA_API_BASE", ""), ("LAMBDA_API_KEY", "")],
    Some(""),
    Some("")
)]
#[case::inception_absent(Host::Inception, &[], None, None)]
#[case::inception_supplied(
    Host::Inception,
    &[
        ("INCEPTION_API_BASE", "https://inception.example/v1"),
        ("INCEPTION_API_KEY", "inception-key"),
    ],
    Some("https://inception.example/v1"),
    Some("inception-key")
)]
#[case::inception_empty_values_stay_present(
    Host::Inception,
    &[("INCEPTION_API_BASE", ""), ("INCEPTION_API_KEY", "")],
    Some(""),
    Some("")
)]
fn loads_each_hosts_environment_values(
    #[case] host: Host,
    #[case] environment: &[(&str, &str)],
    #[case] base: Option<&str>,
    #[case] key: Option<&str>,
) {
    let (loaded_base, loaded_key) = values(host, &load(environment));
    assert_eq!(loaded_base.as_deref(), base);
    assert_eq!(loaded_key.as_deref(), key);
}

#[rstest]
#[case::gradient_base_only(Host::Gradient, &[("GRADIENT_AI_AGENT_ENDPOINT", "https://agent.example")])]
#[case::gradient_key_only(Host::Gradient, &[("GRADIENT_AI_API_KEY", "gradient-key")])]
#[case::heroku_base_only(Host::Heroku, &[("HEROKU_API_BASE", "https://heroku.example")])]
#[case::heroku_key_only(Host::Heroku, &[("HEROKU_API_KEY", "heroku-key")])]
#[case::hyperbolic_base_only(Host::Hyperbolic, &[("HYPERBOLIC_API_BASE", "https://hyperbolic.example/v1")])]
#[case::hyperbolic_key_only(Host::Hyperbolic, &[("HYPERBOLIC_API_KEY", "hyperbolic-key")])]
#[case::lambda_base_only(Host::Lambda, &[("LAMBDA_API_BASE", "https://lambda.example/v1")])]
#[case::lambda_key_only(Host::Lambda, &[("LAMBDA_API_KEY", "lambda-key")])]
#[case::inception_base_only(Host::Inception, &[("INCEPTION_API_BASE", "https://inception.example/v1")])]
#[case::inception_key_only(Host::Inception, &[("INCEPTION_API_KEY", "inception-key")])]
fn a_base_or_a_key_loads_alone(#[case] host: Host, #[case] environment: &[(&str, &str)]) {
    let (base, key) = values(host, &load(environment));
    assert!(base.is_some() ^ key.is_some());
}

const GLOBAL_KEY: &str = "litellm_settings:\n  inception_key: global-key\n";
const GLOBAL_KEY_FROM_ENVIRONMENT: &str =
    "litellm_settings:\n  inception_key: os.environ/INCEPTION_TEAM_KEY\n";

#[rstest]
#[case::literal(GLOBAL_KEY, &[], Some("global-key"))]
#[case::environment_reference(
    GLOBAL_KEY_FROM_ENVIRONMENT,
    &[("INCEPTION_TEAM_KEY", "team-key")],
    Some("team-key")
)]
#[case::empty("litellm_settings:\n  inception_key: ''\n", &[], Some(""))]
#[case::absent("", &[], None)]
#[case::other_settings("litellm_settings:\n  drop_params: true\n", &[], None)]
fn loads_the_inception_global_key(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let loaded = load_with(source, environment);
    assert_eq!(
        loaded
            .inception_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        expected
    );
}

#[test]
fn a_missing_environment_reference_names_the_variable_and_path() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(GLOBAL_KEY_FROM_ENVIRONMENT)
    .expect_err("the referenced variable is absent");
    assert_eq!(
        error.to_string(),
        "environment variable INCEPTION_TEAM_KEY for litellm_settings.inception_key is not set"
    );
    match error {
        ConfigError::MissingEnvironment { variable, path } => {
            assert_eq!(variable, "INCEPTION_TEAM_KEY");
            assert_eq!(path, "litellm_settings.inception_key");
        }
        ConfigError::AgenticLoopCeiling { .. }
        | ConfigError::Environment { .. }
        | ConfigError::Yaml { .. }
        | ConfigError::Unsupported { .. }
        | ConfigError::InvalidFloat { .. }
        | ConfigError::InvalidInteger { .. }
        | ConfigError::InvalidScalar { .. } => panic!("unexpected error"),
    }
}
