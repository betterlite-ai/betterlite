#![cfg(test)]

use betterlite_config::sections::access_windows::{Settings, WindowConfig};
use jiff::tz::TimeZoneDatabase;

use super::deployment_windows;

fn database() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

fn window_config(start: &str, end: &str, timezone: &str, team_ids: &[&str]) -> WindowConfig {
    WindowConfig {
        start: Some(start.to_owned()),
        end: Some(end.to_owned()),
        timezone: Some(timezone.to_owned()),
        team_ids: Some(team_ids.iter().map(|id| (*id).to_owned()).collect()),
    }
}

#[test]
fn configured_windows_parse_into_the_router_types() {
    let section = Settings {
        deployments: vec![
            Some(vec![window_config(
                "09:00",
                "17:00",
                "Europe/Berlin",
                &["t1"],
            )]),
            None,
        ],
    };
    let parsed = deployment_windows(&section, &database()).unwrap();
    let first = parsed.first().unwrap().as_ref().unwrap();
    assert_eq!(first.first().unwrap().timezone, "Europe/Berlin");
    assert_eq!(first.first().unwrap().team_ids, ["t1"]);
    assert!(parsed[1].is_none());
}

#[test]
fn an_invalid_configured_window_names_its_index_and_field() {
    let section = Settings {
        deployments: vec![Some(vec![window_config("25:00", "17:00", "UTC", &["t1"])])],
    };
    let error = deployment_windows(&section, &database()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "model_list.0.model_info.access_windows: 0.start: must be a time like 09:00"
    );
}
