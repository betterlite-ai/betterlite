use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::datadog_logging::error::DatadogError;
use crate::datadog_logging::settings::{
    DD_MAX_BATCH_SIZE, DatadogInit, DatadogSettings, resolve_batch_size,
};

use super::support::{self, Environment};

#[rstest]
#[case::absent(None, DD_MAX_BATCH_SIZE, false)]
#[case::zero(Some("0"), 1, false)]
#[case::negative(Some("-5"), 1, false)]
#[case::one(Some("1"), 1, false)]
#[case::maximum(Some("1000"), DD_MAX_BATCH_SIZE, false)]
#[case::above_maximum(Some("1001"), DD_MAX_BATCH_SIZE, false)]
#[case::far_above_maximum(Some("5000"), DD_MAX_BATCH_SIZE, false)]
#[case::surrounding_spaces(Some(" 42 "), 42, false)]
#[case::leading_plus(Some("+7"), 7, false)]
#[case::invalid_text(Some("many"), DD_MAX_BATCH_SIZE, true)]
#[case::empty_counts_as_unset(Some(""), DD_MAX_BATCH_SIZE, false)]
#[case::positive_overflow(Some("99999999999999999999"), DD_MAX_BATCH_SIZE, false)]
#[case::negative_overflow(Some("-99999999999999999999"), 1, false)]
fn datadog_batch_size_resolution_behavior(
    #[case] raw: Option<&str>,
    #[case] expected: usize,
    #[case] warns: bool,
) {
    let environment = raw.map_or_else(Environment::new, |value| {
        support::environment(&[("DD_BATCH_SIZE", value)])
    });
    let (resolved, warnings) = support::capture(|| resolve_batch_size(&environment));
    assert_eq!(resolved, expected);
    assert_eq!(warnings.count_containing("DD_BATCH_SIZE") > 0, warns);
}

#[test]
fn invalid_dd_batch_size_warns_with_the_variable_and_default() {
    let environment = support::environment(&[("DD_BATCH_SIZE", "many")]);
    let (resolved, warnings) = support::capture(|| resolve_batch_size(&environment));
    assert_eq!(resolved, DD_MAX_BATCH_SIZE);
    assert_eq!(warnings.count_containing("DD_BATCH_SIZE"), 1);
    assert_eq!(warnings.count_containing("many"), 1);
    assert_eq!(warnings.count_containing(&DD_MAX_BATCH_SIZE.to_string()), 1);
}

#[derive(Debug, PartialEq, Eq)]
enum Expected {
    Resolved {
        key: &'static str,
        intake: &'static str,
    },
    MissingApiKey,
    MissingSite,
}

fn init(key: Option<&str>, site: Option<&str>, allow_env_credentials: bool) -> DatadogInit {
    support::init_with(key, site, None, None, allow_env_credentials)
}

#[rstest]
#[case::explicit_beats_the_environment(
    Some("explicit-key"),
    Some("explicit.datadoghq.com"),
    &[("DD_API_KEY", "env-key"), ("DD_SITE", "env.datadoghq.com")],
    true,
    Expected::Resolved {
        key: "explicit-key",
        intake: "https://http-intake.logs.explicit.datadoghq.com/api/v2/logs",
    }
)]
#[case::an_explicit_key_takes_the_environment_site(
    Some("explicit-key"),
    None,
    &[("DD_API_KEY", "env-key"), ("DD_SITE", "env.datadoghq.com")],
    true,
    Expected::Resolved {
        key: "explicit-key",
        intake: "https://http-intake.logs.env.datadoghq.com/api/v2/logs",
    }
)]
#[case::the_environment_supplies_both(
    None,
    None,
    &[("DD_API_KEY", "env-key"), ("DD_SITE", "env.datadoghq.com")],
    true,
    Expected::Resolved {
        key: "env-key",
        intake: "https://http-intake.logs.env.datadoghq.com/api/v2/logs",
    }
)]
#[case::environment_credentials_off_hides_the_key(
    None,
    None,
    &[("DD_API_KEY", "env-key"), ("DD_SITE", "env.datadoghq.com")],
    false,
    Expected::MissingApiKey
)]
#[case::environment_credentials_off_keeps_the_explicit_key(
    Some("explicit-key"),
    None,
    &[("DD_API_KEY", "env-key"), ("DD_SITE", "env.datadoghq.com")],
    false,
    Expected::Resolved {
        key: "explicit-key",
        intake: "https://http-intake.logs.env.datadoghq.com/api/v2/logs",
    }
)]
#[case::the_site_is_required(
    Some("explicit-key"),
    None,
    &[],
    true,
    Expected::MissingSite
)]
#[case::the_key_is_required(None, Some("us5.datadoghq.com"), &[], true, Expected::MissingApiKey)]
#[case::an_empty_environment_key_counts_as_unset(
    None,
    Some("us5.datadoghq.com"),
    &[("DD_API_KEY", "")],
    true,
    Expected::MissingApiKey
)]
fn datadog_credentials_resolution_behavior(
    #[case] key: Option<&str>,
    #[case] site: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] allow_env_credentials: bool,
    #[case] expected: Expected,
) {
    let variables = support::environment(environment);
    let resolved = DatadogSettings::resolve(&init(key, site, allow_env_credentials), &variables);
    match (resolved, expected) {
        (
            Ok(settings),
            Expected::Resolved {
                key: wanted,
                intake,
            },
        ) => {
            assert_eq!(settings.intake_url, intake);
            let api_key = settings.api_key.expect("the resolved logger carries a key");
            assert_eq!(api_key.expose_secret(), wanted);
        }
        (Err(DatadogError::MissingApiKey), Expected::MissingApiKey)
        | (Err(DatadogError::MissingSite), Expected::MissingSite) => {}
        (outcome, wanted) => panic!("unexpected {outcome:?} for {wanted:?}"),
    }
}

#[rstest]
#[case::explicit_agent_host_and_port(Some("agent.local"), Some("9000"), &[], "http://agent.local:9000/api/v2/logs")]
#[case::the_agent_port_defaults(Some("agent.local"), None, &[], "http://agent.local:10518/api/v2/logs")]
#[case::the_environment_supplies_the_agent(
    None,
    None,
    &[("LITELLM_DD_AGENT_HOST", "env-agent.local"), ("LITELLM_DD_AGENT_PORT", "8125")],
    "http://env-agent.local:8125/api/v2/logs"
)]
#[case::explicit_agent_values_beat_the_environment(
    Some("explicit.local"),
    Some("7000"),
    &[("LITELLM_DD_AGENT_HOST", "env-agent.local"), ("LITELLM_DD_AGENT_PORT", "8125")],
    "http://explicit.local:7000/api/v2/logs"
)]
#[case::an_empty_environment_port_defaults(
    Some("agent.local"),
    None,
    &[("LITELLM_DD_AGENT_PORT", "")],
    "http://agent.local:10518/api/v2/logs"
)]
#[case::direct_api_uses_the_given_site(
    None,
    None,
    &[],
    "https://http-intake.logs.us5.datadoghq.com/api/v2/logs"
)]
#[case::base_url_overrides_the_agent(
    Some("agent.local"),
    None,
    &[("DD_BASE_URL", "http://localhost:9000")],
    "http://localhost:9000/api/v2/logs"
)]
#[case::base_url_overrides_the_direct_api(
    None,
    None,
    &[("DD_BASE_URL", "http://localhost:9000")],
    "http://localhost:9000/api/v2/logs"
)]
#[case::an_empty_base_url_is_unset(
    None,
    None,
    &[("DD_BASE_URL", "")],
    "https://http-intake.logs.us5.datadoghq.com/api/v2/logs"
)]
#[case::the_base_url_is_joined_verbatim(
    None,
    None,
    &[("DD_BASE_URL", "http://localhost:9000/")],
    "http://localhost:9000//api/v2/logs"
)]
fn datadog_endpoint_configuration_behavior(
    #[case] host: Option<&str>,
    #[case] port: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    let init = support::init_with(Some("key-1"), Some("us5.datadoghq.com"), host, port, true);
    let settings =
        DatadogSettings::resolve(&init, &support::environment(environment)).expect("resolves");
    assert_eq!(settings.intake_url, expected);
}

#[test]
fn datadog_endpoint_configuration_takes_the_environment_site() {
    let variables = support::environment(&[("DD_SITE", "eu.datadoghq.com")]);
    let settings = DatadogSettings::resolve(&init(Some("key-1"), None, true), &variables)
        .expect("the settings resolve");
    assert_eq!(
        settings.intake_url,
        "https://http-intake.logs.eu.datadoghq.com/api/v2/logs"
    );
}

#[rstest]
#[case::absent(&[], Some(Duration::from_secs(5)))]
#[case::configured(&[("DEFAULT_FLUSH_INTERVAL_SECONDS", "30")], Some(Duration::from_secs(30)))]
#[case::zero_is_used_as_given(&[("DEFAULT_FLUSH_INTERVAL_SECONDS", "0")], Some(Duration::ZERO))]
#[case::empty_counts_as_unset(&[("DEFAULT_FLUSH_INTERVAL_SECONDS", "")], Some(Duration::from_secs(5)))]
#[case::not_a_number(&[("DEFAULT_FLUSH_INTERVAL_SECONDS", "soon")], None)]
fn datadog_flush_interval_resolution_behavior(
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<Duration>,
) {
    let resolved = DatadogSettings::resolve(
        &init(Some("key-1"), Some("us5.datadoghq.com"), true),
        &support::environment(environment),
    );
    match (resolved, expected) {
        (Ok(settings), Some(interval)) => assert_eq!(settings.batch.flush_interval, interval),
        (Err(DatadogError::Environment(_)), None) => {}
        (outcome, wanted) => panic!("unexpected {outcome:?} for {wanted:?}"),
    }
}
