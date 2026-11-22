#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{Settings, UserHeaderMapping, UserHeaderRole};

fn load(source: &str) -> Result<Settings, ConfigError> {
    let lookup = BTreeMap::from([
        ("FORWARD_ON".to_owned(), "yes".to_owned()),
        ("FORWARD_EMPTY".to_owned(), String::new()),
        ("FORWARD_BAD".to_owned(), "maybe".to_owned()),
        ("USER_HEADER_FROM_ENV".to_owned(), "X-From-Env".to_owned()),
        (
            "MAPPED_HEADER_FROM_ENV".to_owned(),
            "X-Mapped-Env".to_owned(),
        ),
    ]);
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .request_headers)
}

fn mapping(header_name: &str, litellm_user_role: UserHeaderRole) -> UserHeaderMapping {
    UserHeaderMapping {
        header_name: header_name.to_owned(),
        litellm_user_role,
    }
}

#[derive(Debug)]
enum Expected {
    Loaded(Settings),
    InvalidScalar(&'static str),
    Environment,
    MissingEnvironment(&'static str, &'static str),
    Yaml,
}

const ALL_KEYS: &str = "general_settings:
  user_header_mappings:
    - {header_name: X-OpenWebUI-User-Id, litellm_user_role: internal_user}
    - {header_name: X-End-User, litellm_user_role: customer}
  user_header_name: X-User
  forward_openai_org_id: true
  forward_client_headers_to_llm_api: true
litellm_settings:
  add_user_information_to_llm_headers: true
  model_group_settings:
    forward_client_headers_to_llm_api: [gpt-4o, team/*]
  forward_traceparent_to_llm_provider: true
";

fn all_keys() -> Settings {
    Settings {
        user_header_mappings: vec![
            mapping("X-OpenWebUI-User-Id", UserHeaderRole::InternalUser),
            mapping("X-End-User", UserHeaderRole::Customer),
        ],
        user_header_name: Some("X-User".to_owned()),
        add_user_information: true,
        forward_openai_org_id: true,
        model_group_forwarding: Some(vec!["gpt-4o".to_owned(), "team/*".to_owned()]),
        forward_client_headers: true,
        forward_traceparent: true,
    }
}

fn only(change: fn(&mut Settings)) -> Settings {
    let mut settings = Settings::default();
    change(&mut settings);
    settings
}

#[rstest]
#[case::unset("", Expected::Loaded(Settings::default()))]
#[case::every_key(ALL_KEYS, Expected::Loaded(all_keys()))]
#[case::explicit_nulls(
    "general_settings: {user_header_mappings: null, user_header_name: null, forward_openai_org_id: null, forward_client_headers_to_llm_api: null}\nlitellm_settings: {add_user_information_to_llm_headers: null, model_group_settings: null, forward_traceparent_to_llm_provider: null}\n",
    Expected::Loaded(Settings::default())
)]
#[case::explicit_false(
    "general_settings: {forward_openai_org_id: false, forward_client_headers_to_llm_api: false}\nlitellm_settings: {add_user_information_to_llm_headers: false, forward_traceparent_to_llm_provider: false}\n",
    Expected::Loaded(Settings::default())
)]
#[case::user_header_mappings_empty(
    "general_settings: {user_header_mappings: []}\n",
    Expected::Loaded(Settings::default())
)]
#[case::user_header_mappings_bad_role(
    "general_settings: {user_header_mappings: [{header_name: X-User, litellm_user_role: proxy_admin}]}\n",
    Expected::Yaml
)]
#[case::user_header_mappings_bare_mapping(
    "general_settings: {user_header_mappings: {header_name: X-User, litellm_user_role: internal_user}}\n",
    Expected::Yaml
)]
#[case::user_header_name_empty(
    "general_settings: {user_header_name: \"\"}\n",
    Expected::Loaded(only(|settings| settings.user_header_name = Some(String::new())))
)]
#[case::user_header_name_mapping("general_settings: {user_header_name: {a: b}}\n", Expected::Yaml)]
#[case::add_user_information_from_environment(
    "litellm_settings: {add_user_information_to_llm_headers: os.environ/FORWARD_ON}\n",
    Expected::Loaded(only(|settings| settings.add_user_information = true))
)]
#[case::add_user_information_text(
    "litellm_settings: {add_user_information_to_llm_headers: \"true\"}\n",
    Expected::InvalidScalar("litellm_settings.add_user_information_to_llm_headers")
)]
#[case::forward_openai_org_id_empty_environment(
    "general_settings: {forward_openai_org_id: os.environ/FORWARD_EMPTY}\n",
    Expected::Loaded(Settings::default())
)]
#[case::forward_openai_org_id_text(
    "general_settings: {forward_openai_org_id: \"yes\"}\n",
    Expected::InvalidScalar("general_settings.forward_openai_org_id")
)]
#[case::model_group_settings_empty_list(
    "litellm_settings: {model_group_settings: {forward_client_headers_to_llm_api: []}}\n",
    Expected::Loaded(only(|settings| settings.model_group_forwarding = Some(Vec::new())))
)]
#[case::model_group_settings_without_list(
    "litellm_settings: {model_group_settings: {}}\n",
    Expected::Loaded(Settings::default())
)]
#[case::model_group_settings_text(
    "litellm_settings: {model_group_settings: {forward_client_headers_to_llm_api: gpt-4o}}\n",
    Expected::Yaml
)]
#[case::forward_client_headers_bad_environment(
    "general_settings: {forward_client_headers_to_llm_api: os.environ/FORWARD_BAD}\n",
    Expected::Environment
)]
#[case::forward_client_headers_text(
    "general_settings: {forward_client_headers_to_llm_api: \"off\"}\n",
    Expected::InvalidScalar("general_settings.forward_client_headers_to_llm_api")
)]
#[case::forward_traceparent_missing_environment(
    "litellm_settings: {forward_traceparent_to_llm_provider: os.environ/FORWARD_UNSET}\n",
    Expected::MissingEnvironment(
        "FORWARD_UNSET",
        "litellm_settings.forward_traceparent_to_llm_provider"
    )
)]
#[case::user_header_name_from_environment(
    "general_settings: {user_header_name: os.environ/USER_HEADER_FROM_ENV}\n",
    Expected::Loaded(only(|settings| settings.user_header_name = Some("X-From-Env".to_owned())))
)]
#[case::user_header_name_missing_environment(
    "general_settings: {user_header_name: os.environ/USER_HEADER_UNSET}\n",
    Expected::MissingEnvironment("USER_HEADER_UNSET", "general_settings.user_header_name")
)]
#[case::user_header_mapping_from_environment(
    "general_settings: {user_header_mappings: [{header_name: os.environ/MAPPED_HEADER_FROM_ENV, litellm_user_role: internal_user}]}\n",
    Expected::Loaded(only(|settings| settings.user_header_mappings =
        vec![mapping("X-Mapped-Env", UserHeaderRole::InternalUser)]))
)]
#[case::user_header_mapping_missing_environment(
    "general_settings: {user_header_mappings: [{header_name: os.environ/MAPPED_HEADER_UNSET, litellm_user_role: customer}]}\n",
    Expected::MissingEnvironment(
        "MAPPED_HEADER_UNSET",
        "general_settings.user_header_mappings[0].header_name"
    )
)]
#[case::second_user_header_mapping_path(
    "general_settings: {user_header_mappings: [{header_name: X-First, litellm_user_role: internal_user}, {header_name: os.environ/MAPPED_UNSET, litellm_user_role: customer}]}\n",
    Expected::MissingEnvironment(
        "MAPPED_UNSET",
        "general_settings.user_header_mappings[1].header_name"
    )
)]
#[case::forward_traceparent_from_environment(
    "litellm_settings: {forward_traceparent_to_llm_provider: os.environ/FORWARD_ON}\n",
    Expected::Loaded(only(|settings| settings.forward_traceparent = true))
)]
#[case::forward_traceparent_number(
    "litellm_settings: {forward_traceparent_to_llm_provider: 1}\n",
    Expected::Yaml
)]
fn loads_settings(#[case] source: &str, #[case] expected: Expected) {
    match (load(source), expected) {
        (Ok(settings), Expected::Loaded(want)) => assert_eq!(settings, want),
        (Err(ConfigError::InvalidScalar { path, value }), Expected::InvalidScalar(want)) => {
            assert_eq!((path.as_str(), value), (want, false));
        }
        (Err(ConfigError::Environment { .. }), Expected::Environment)
        | (Err(ConfigError::Yaml { .. }), Expected::Yaml) => {}
        (
            Err(ConfigError::MissingEnvironment { variable, path }),
            Expected::MissingEnvironment(want_variable, want_path),
        ) => assert_eq!(
            (variable.as_str(), path.as_str()),
            (want_variable, want_path)
        ),
        (other, want) => panic!("{want:?} expected, got {other:?}"),
    }
}
