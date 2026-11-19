#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::access_windows::{Settings, WindowConfig};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.access_windows)
        .map_err(|error| error.to_string())
}

fn window(start: &str, end: &str, timezone: &str, team_ids: &[&str]) -> WindowConfig {
    WindowConfig {
        start: Some(start.to_owned()),
        end: Some(end.to_owned()),
        timezone: Some(timezone.to_owned()),
        team_ids: Some(team_ids.iter().map(|id| (*id).to_owned()).collect()),
    }
}

#[test]
fn reads_each_deployments_windows() {
    let source = "model_list:\n  - model_name: first\n    model_info:\n      access_windows:\n        - {start: \"09:00\", end: \"17:00\", timezone: Europe/Berlin, team_ids: [t1]}\n        - {start: \"22:00\", end: \"06:00\", timezone: UTC, team_ids: [t2, t3]}\n  - model_name: second\n";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments,
        [
            Some(vec![
                window("09:00", "17:00", "Europe/Berlin", &["t1"]),
                window("22:00", "06:00", "UTC", &["t2", "t3"]),
            ]),
            None,
        ]
    );
}

#[test]
fn resolves_environment_references() {
    let source = "model_list:\n  - model_name: probe\n    model_info:\n      access_windows:\n        - {start: os.environ/START, end: os.environ/END, timezone: os.environ/ZONE, team_ids: [os.environ/TEAM]}\n";
    let resolved = settings(
        source,
        &[
            ("START", "09:00"),
            ("END", "17:00"),
            ("ZONE", "Europe/Berlin"),
            ("TEAM", "t1"),
        ],
    )
    .unwrap();
    assert_eq!(
        resolved.deployments,
        [Some(vec![window(
            "09:00",
            "17:00",
            "Europe/Berlin",
            &["t1"]
        )])]
    );
}

#[test]
fn a_missing_environment_variable_names_its_path() {
    let source = "model_list:\n  - model_name: probe\n    model_info:\n      access_windows:\n        - {start: os.environ/START, end: \"17:00\", timezone: UTC, team_ids: [t1]}\n";
    let error = settings(source, &[]).unwrap_err();
    assert!(
        error.contains("model_list.0.model_info.access_windows.0.start"),
        "{error}"
    );
    assert!(error.contains("START"), "{error}");
}

#[test]
fn a_null_value_has_no_windows() {
    let source = "model_list:\n  - model_name: probe\n    model_info:\n      access_windows:\n";
    assert_eq!(settings(source, &[]).unwrap().deployments, [None]);
}

#[test]
fn an_empty_window_list_stays_configured() {
    let source = "model_list:\n  - model_name: probe\n    model_info:\n      access_windows: []\n";
    assert_eq!(
        settings(source, &[]).unwrap().deployments,
        [Some(Vec::new())]
    );
}

#[test]
fn an_unknown_window_field_is_refused() {
    let source = "model_list:\n  - model_name: probe\n    model_info:\n      access_windows:\n        - {start: \"09:00\", end: \"17:00\", timezone: UTC, team_ids: [t1], colour: blue}\n";
    settings(source, &[]).unwrap_err();
}
