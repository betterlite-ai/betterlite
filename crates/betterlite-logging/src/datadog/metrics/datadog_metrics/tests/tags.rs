use crate::datadog::metrics::tags::extract_tags;
use crate::fields::payload::StandardLoggingPayload;

use super::support;

fn tags_with<Configure>(
    environment: &support::Environment,
    configure: Configure,
    status_code: Option<&str>,
) -> Vec<String>
where
    Configure: FnOnce(&mut StandardLoggingPayload),
{
    let (logger, _transport) =
        support::build(environment, support::MetricsInit::default(), Vec::new());
    let mut payload = support::payload();
    configure(&mut payload);
    extract_tags(&logger.settings.environment, &payload, status_code)
}

#[test]
fn tags_use_the_environment_defaults() {
    let tags = tags_with(&support::environment(&[]), |_payload| {}, None);
    assert_eq!(
        tags,
        vec![
            "env:unknown".to_owned(),
            "service:litellm-server".to_owned(),
            "version:unknown".to_owned(),
            "HOSTNAME:".to_owned(),
            "POD_NAME:unknown".to_owned(),
            "provider:openai".to_owned(),
            "model_name:gpt-4o".to_owned(),
            "model_group:production".to_owned(),
        ]
    );
}

#[test]
fn tags_use_the_environment_values() {
    let environment = support::environment(&[
        ("DD_ENV", "production"),
        ("DD_SERVICE", "gateway"),
        ("DD_VERSION", "1.2.3"),
        ("HOSTNAME", "host-1"),
        ("POD_NAME", "pod-1"),
    ]);
    let tags = tags_with(&environment, |_payload| {}, None);
    assert_eq!(tags[0], "env:production");
    assert_eq!(tags[1], "service:gateway");
    assert_eq!(tags[2], "version:1.2.3");
    assert_eq!(tags[3], "HOSTNAME:host-1");
    assert_eq!(tags[4], "POD_NAME:pod-1");
}

#[test]
fn tags_treat_an_empty_environment_value_as_unset() {
    let environment = support::environment(&[("DD_ENV", ""), ("DD_SERVICE", "")]);
    let tags = tags_with(&environment, |_payload| {}, None);
    assert_eq!(tags[0], "env:unknown");
    assert_eq!(tags[1], "service:litellm-server");
}

#[test]
fn tags_add_the_status_code_only_when_given() {
    let without = tags_with(&support::environment(&[]), |_payload| {}, None);
    assert!(!without.iter().any(|tag| tag.starts_with("status_code:")));
    let with = tags_with(&support::environment(&[]), |_payload| {}, Some("429"));
    assert!(with.contains(&"status_code:429".to_owned()));
}

#[test]
fn tags_skip_an_empty_provider_model_and_model_group() {
    let tags = tags_with(
        &support::environment(&[]),
        |payload| {
            payload.custom_llm_provider = Some(String::new());
            payload.model = String::new();
            payload.model_group = Some(String::new());
        },
        None,
    );
    assert!(!tags.iter().any(|tag| tag.starts_with("provider:")));
    assert!(!tags.iter().any(|tag| tag.starts_with("model_name:")));
    assert!(!tags.iter().any(|tag| tag.starts_with("model_group:")));
}

#[test]
fn tags_skip_a_missing_model_group() {
    let tags = tags_with(
        &support::environment(&[]),
        |payload| payload.model_group = None,
        None,
    );
    assert!(!tags.iter().any(|tag| tag.starts_with("model_group:")));
}

#[test]
fn tags_prefer_the_team_alias_over_the_team_id() {
    let tags = tags_with(
        &support::environment(&[]),
        |payload| {
            payload.metadata.user_api_key_team_alias = Some("Team One".to_owned());
            payload.metadata.user_api_key_team_id = Some("team-id".to_owned());
        },
        None,
    );
    assert!(tags.contains(&"team:team_one".to_owned()));
}

#[test]
fn tags_fall_through_an_empty_team_alias_to_the_team_id() {
    let tags = tags_with(
        &support::environment(&[]),
        |payload| {
            payload.metadata.user_api_key_team_alias = Some(String::new());
            payload.metadata.team_alias = Some(String::new());
            payload.metadata.user_api_key_team_id = Some("team-id".to_owned());
        },
        None,
    );
    assert!(tags.contains(&"team:team-id".to_owned()));
}

#[test]
fn tags_read_the_plain_team_alias_last() {
    let tags = tags_with(
        &support::environment(&[]),
        |payload| {
            payload.metadata.team_alias = Some("Second Team".to_owned());
            payload.metadata.team_id = Some("team-id".to_owned());
        },
        None,
    );
    assert!(tags.contains(&"team:second_team".to_owned()));
}

#[test]
fn tags_normalize_the_team_value() {
    let tags = tags_with(
        &support::environment(&[]),
        |payload| {
            payload.metadata.team_id = Some("Team One / Prod!".to_owned());
        },
        None,
    );
    assert!(tags.contains(&"team:team_one_/_prod".to_owned()));
}
