#![cfg(test)]

use super::AudioSession;
use crate::audio::convert::AudioSettings;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_setup::RequestMap;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;
use std::time::Duration;

fn session(environment: BTreeMap<String, String>) -> AudioSession {
    AudioSession {
        environment,
        client: crate::completion::setup::outbound_client(&BTreeMap::new()).unwrap(),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    }
}

fn environment_of(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn request_credentials_read_the_environment() {
    let found = session(environment_of(&[
        ("OPENAI_BASE_URL", "https://env-root/v1"),
        ("OPENAI_API_BASE", "https://env-base/v1"),
        ("OPENAI_ORGANIZATION", "org-env"),
        ("OPENAI_PROJECT", "project-env"),
        ("OPENAI_API_KEY", "sk-env"),
    ]))
    .credentials(&AudioSettings::default(), &RequestMap::new())
    .unwrap();
    assert_eq!(found.api_base, "https://env-root/v1");
    assert_eq!(found.organization.as_deref(), Some("org-env"));
    assert_eq!(found.project.as_deref(), Some("project-env"));
    assert_eq!(found.api_key.unwrap().expose_secret(), "sk-env");
}

#[test]
fn request_credentials_skip_an_empty_environment_value() {
    let found = session(environment_of(&[
        ("OPENAI_BASE_URL", ""),
        ("OPENAI_API_BASE", "https://env-base/v1"),
        ("OPENAI_ORGANIZATION", ""),
    ]))
    .credentials(&AudioSettings::default(), &RequestMap::new())
    .unwrap();
    assert_eq!(found.api_base, "https://env-base/v1");
    assert_eq!(found.organization, None);
}

#[test]
fn request_credentials_skip_an_empty_argument() {
    let arguments = RequestMap::from([
        (String::from("api_base"), Json::Text(String::new())),
        (
            String::from("base_url"),
            Json::Text(String::from("https://base/v1")),
        ),
        (String::from("project"), Json::Text(String::new())),
    ]);
    let found = session(environment_of(&[
        ("OPENAI_BASE_URL", "https://env-root/v1"),
        ("OPENAI_PROJECT", "project-env"),
    ]))
    .credentials(&AudioSettings::default(), &arguments)
    .unwrap();
    assert_eq!(found.api_base, "https://base/v1");
    assert_eq!(found.project.as_deref(), Some("project-env"));
}

#[test]
fn request_timeout_reads_the_environment() {
    assert_eq!(
        session(environment_of(&[("REQUEST_TIMEOUT", "17")])).request_timeout(),
        Duration::from_secs(17)
    );
    assert_eq!(
        session(BTreeMap::new()).request_timeout(),
        Duration::from_secs(6000)
    );
}
