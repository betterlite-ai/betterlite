#![cfg(test)]

use betterlite_config::sections::access_windows::{Settings, WindowConfig};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use super::{Windows, deployment_windows};

fn database() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn window_config(start: &str, end: &str, timezone: &str, team_ids: &[&str]) -> WindowConfig {
    WindowConfig {
        start: Some(start.to_owned()),
        end: Some(end.to_owned()),
        timezone: Some(timezone.to_owned()),
        team_ids: Some(team_ids.iter().map(|id| (*id).to_owned()).collect()),
    }
}

fn spec(id: &str, access_windows: Json) -> DeploymentSpec {
    let model_info = JsonObject::from_iter([
        ("id".to_owned(), Json::Text(id.to_owned())),
        ("access_windows".to_owned(), access_windows),
    ]);
    DeploymentSpec::new(
        "gpt".to_owned(),
        JsonObject::new(),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn window(start: &str, end: &str, timezone: &str, team_ids: &[&str]) -> Json {
    Json::Object(JsonObject::from_iter([
        ("start".to_owned(), Json::Text(start.to_owned())),
        ("end".to_owned(), Json::Text(end.to_owned())),
        ("timezone".to_owned(), Json::Text(timezone.to_owned())),
        (
            "team_ids".to_owned(),
            Json::List(
                team_ids
                    .iter()
                    .map(|id| Json::Text((*id).to_owned()))
                    .collect(),
            ),
        ),
    ]))
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
    assert!(parsed.get(1).unwrap().is_none());
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

#[test]
fn the_reservation_filter_refuses_a_pool_emptied_by_a_window() {
    let deployments = vec![spec(
        "d",
        Json::List(vec![window("22:00", "06:00", "UTC", &["mine"])]),
    )];
    let windows = Windows {
        now: at("2026-01-05T23:00:00Z"),
        database: &database(),
    };
    let error = windows
        .reserved("gpt", deployments, Some("other"))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Deployment gpt is reserved for another team until 06:00 UTC"
    );
}

#[test]
fn the_reservation_filter_keeps_a_pool_the_callers_team_holds() {
    let deployments = vec![spec(
        "d",
        Json::List(vec![window("22:00", "06:00", "UTC", &["mine"])]),
    )];
    let database = database();
    let windows = Windows {
        now: at("2026-01-05T23:00:00Z"),
        database: &database,
    };
    let kept = windows.reserved("gpt", deployments, Some("mine")).unwrap();
    assert_eq!(kept.len(), 1);
}
