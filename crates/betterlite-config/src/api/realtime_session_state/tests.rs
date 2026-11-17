#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

use super::{LOGGED_EVENT_TYPES_PATH, LoggedEventTypes};

fn load(source: &str) -> Result<Config, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
}

#[rstest]
#[case::absent("{}\n", LoggedEventTypes::Default)]
#[case::explicit_null(
    "litellm_settings:\n  logged_real_time_event_types: null\n",
    LoggedEventTypes::Default
)]
#[case::wildcard(
    "litellm_settings:\n  logged_real_time_event_types: \"*\"\n",
    LoggedEventTypes::All
)]
#[case::empty_list(
    "litellm_settings:\n  logged_real_time_event_types: []\n",
    LoggedEventTypes::Only(Vec::new())
)]
#[case::one_event(
    "litellm_settings:\n  logged_real_time_event_types: [response.done]\n",
    LoggedEventTypes::Only(vec!["response.done".to_owned()])
)]
#[case::several_events(
    "litellm_settings:\n  logged_real_time_event_types: [session.created, response.done]\n",
    LoggedEventTypes::Only(vec!["session.created".to_owned(), "response.done".to_owned()])
)]
fn loads_realtime_logged_events(#[case] source: &str, #[case] expected: LoggedEventTypes) {
    let config = load(source).expect("the event selection is a supported configuration key");
    assert_eq!(
        config
            .sections
            .realtime_session_state
            .logged_real_time_event_types,
        expected
    );
}

#[rstest]
#[case::number("litellm_settings:\n  logged_real_time_event_types: 1\n", false)]
#[case::true_flag("litellm_settings:\n  logged_real_time_event_types: true\n", true)]
#[case::false_flag("litellm_settings:\n  logged_real_time_event_types: false\n", false)]
#[case::other_text(
    "litellm_settings:\n  logged_real_time_event_types: session.created\n",
    false
)]
#[case::empty_text("litellm_settings:\n  logged_real_time_event_types: \"\"\n", false)]
#[case::object(
    "litellm_settings:\n  logged_real_time_event_types: {type: response.done}\n",
    false
)]
#[case::list_of_numbers(
    "litellm_settings:\n  logged_real_time_event_types: [response.done, 1]\n",
    false
)]
fn rejects_other_realtime_logged_events(#[case] source: &str, #[case] flagged: bool) {
    let error = load(source).expect_err("only null, a string list and \"*\" select logged events");
    assert_eq!(error.category(), "invalid_scalar");
    match error {
        ConfigError::InvalidScalar { path, value } => {
            assert_eq!(path, LOGGED_EVENT_TYPES_PATH);
            assert_eq!(value, flagged);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[rstest]
#[case::wildcard(
    "os.environ/REALTIME_EVENTS",
    BTreeMap::from([("REALTIME_EVENTS".to_owned(), "*".to_owned())]),
    LoggedEventTypes::All
)]
#[case::empty_is_unset(
    "os.environ/REALTIME_EVENTS",
    BTreeMap::from([("REALTIME_EVENTS".to_owned(), String::new())]),
    LoggedEventTypes::Default
)]
#[case::list_item(
    "[os.environ/REALTIME_EVENT]",
    BTreeMap::from([("REALTIME_EVENT".to_owned(), "response.done".to_owned())]),
    LoggedEventTypes::Only(vec!["response.done".to_owned()])
)]
fn reads_an_environment_reference_in_logged_events(
    #[case] reference: &str,
    #[case] lookup: BTreeMap<String, String>,
    #[case] expected: LoggedEventTypes,
) {
    let source = format!("litellm_settings:\n  logged_real_time_event_types: {reference}\n");
    let config = ConfigLoader { lookup }
        .load(&source)
        .expect("an environment reference is read into the selection");
    assert_eq!(
        config
            .sections
            .realtime_session_state
            .logged_real_time_event_types,
        expected
    );
}

#[test]
fn rejects_an_environment_selection_that_is_not_a_wildcard() {
    let error = ConfigLoader {
        lookup: BTreeMap::from([("REALTIME_EVENTS".to_owned(), "session.created".to_owned())]),
    }
    .load("litellm_settings:\n  logged_real_time_event_types: os.environ/REALTIME_EVENTS\n")
    .expect_err("a resolved event name is not the setting's type");
    assert_eq!(error.category(), "invalid_scalar");
    match error {
        ConfigError::InvalidScalar { path, value } => {
            assert_eq!(path, LOGGED_EVENT_TYPES_PATH);
            assert!(!value);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn loads_realtime_logged_events_rejects_an_unset_environment() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  logged_real_time_event_types: os.environ/REALTIME_EVENTS\n")
    .expect_err("an unset environment variable is a startup configuration error");
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "REALTIME_EVENTS" && path == LOGGED_EVENT_TYPES_PATH
    ));
}
