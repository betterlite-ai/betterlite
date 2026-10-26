#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{Environment, RequestTagValue, TeamMetadata, normalize_value, tags};

#[rstest]
#[case::trims_and_lowercases(" Foo.Bar ", "foo.bar")]
#[case::collapses_underscores("A__B / C", "a_b_/_c")]
#[case::keeps_passthrough("x-y:z/w.v", "x-y:z/w.v")]
#[case::keeps_unicode_letters("__\u{c4}__", "\u{e4}")]
#[case::replaces_combining_marks("a\u{345}b", "a_b")]
#[case::replaces_symbols("a!@b", "a_b")]
fn datadog_tag_normalization_behavior(#[case] value: &str, #[case] expected: &str) {
    assert_eq!(normalize_value(value), expected);
}

#[rstest]
#[case::unset(&[], false)]
#[case::true_any_case(&[("USE_DDTRACE", " TRUE "), ("USE_DDPROFILER", "true")], true)]
#[case::one_is_not_true(&[("USE_DDTRACE", "1"), ("USE_DDPROFILER", "yes")], false)]
fn datadog_environment_defaults_behavior(#[case] pairs: &[(&str, &str)], #[case] enabled: bool) {
    let values = pairs
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)));
    let environment = Environment::from_lookup(&BTreeMap::from_iter(values));
    let expected = Environment {
        source: String::from("litellm"),
        service: String::from("litellm-server"),
        hostname: String::new(),
        env: String::from("unknown"),
        pod_name: String::from("unknown"),
        version: String::from("unknown"),
        base_url: None,
        use_tracer: enabled,
        use_profiler: enabled,
    };
    assert_eq!(environment, expected);
}

#[test]
fn datadog_environment_defaults_behavior_reads_every_variable() {
    let names = "DD_SOURCE DD_SERVICE HOSTNAME DD_ENV POD_NAME DD_VERSION DD_BASE_URL";
    let values = names
        .split(' ')
        .map(|name| (String::from(name), name.to_lowercase()));
    let environment = Environment::from_lookup(&BTreeMap::from_iter(values));
    let metadata = TeamMetadata {
        user_api_key_team_alias: Some(String::new()),
        team_alias: None,
        user_api_key_team_id: Some(String::from("Team One")),
        team_id: Some(String::from("ignored")),
    };
    assert_eq!(environment.source, "dd_source");
    assert_eq!(environment.base_url.as_deref(), Some("dd_base_url"));
    let joined = tags(
        &environment,
        &[RequestTagValue::Text(String::from("A B"))],
        &metadata,
    )
    .join(",");
    let expected =
        "env:dd_env,service:dd_service,version:dd_version,HOSTNAME:hostname,POD_NAME:pod_name";
    assert_eq!(joined, format!("{expected},request_tag:a_b,team:team_one"));
}

#[rstest]
#[case::none(RequestTagValue::Null, "request_tag:none")]
#[case::boolean(RequestTagValue::Bool(true), "request_tag:true")]
#[case::number(RequestTagValue::Number(String::from("123")), "request_tag:123")]
fn datadog_request_tag_values_match_litellm(
    #[case] value: RequestTagValue,
    #[case] expected: &str,
) {
    let environment = Environment::from_lookup(&BTreeMap::new());
    let metadata = TeamMetadata {
        user_api_key_team_alias: None,
        team_alias: None,
        user_api_key_team_id: None,
        team_id: None,
    };
    assert!(
        tags(&environment, &[value], &metadata)
            .iter()
            .any(|tag| tag == expected)
    );
}
