#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load(source)?.sections.login_return)
}

#[test]
fn return_url_default() {
    assert_eq!(load("", &[]).unwrap().control_plane_url, None);
}

#[test]
fn return_url_loaded() {
    let settings = load(
        "general_settings:\n  control_plane_url: https://control.example\n",
        &[],
    )
    .unwrap();
    assert_eq!(
        settings.control_plane_url.as_deref(),
        Some("https://control.example")
    );
}

#[test]
fn return_url_null() {
    let settings = load("general_settings:\n  control_plane_url: null\n", &[]).unwrap();
    assert_eq!(settings.control_plane_url, None);
}

#[test]
fn return_url_from_environment() {
    let settings = load(
        "general_settings:\n  control_plane_url: os.environ/CONTROL_PLANE_URL\n",
        &[("CONTROL_PLANE_URL", "https://env.example")],
    )
    .unwrap();
    assert_eq!(
        settings.control_plane_url.as_deref(),
        Some("https://env.example")
    );
}
