#![cfg(test)]

use chrono::{DateTime, Utc};
use jiff::civil::Time;
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::access_windows::reservation::{filter_reserved_deployments, reserved_deployments};
use crate::access_windows::team::filter_team_based_models;
use crate::access_windows::window::{
    ModelAccessWindow, access_windows_config_error, is_window_active, parse_access_windows,
};
use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_catalog::types::Authority;
use crate::deployment_identity::spec::DeploymentSpec;

fn database() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn window(start: &str, end: &str, timezone: &str, team_ids: &[&str]) -> Json {
    Json::Object(JsonObject::from_iter([
        ("start".to_owned(), text(start)),
        ("end".to_owned(), text(end)),
        ("timezone".to_owned(), text(timezone)),
        (
            "team_ids".to_owned(),
            Json::List(team_ids.iter().map(|id| text(id)).collect()),
        ),
    ]))
}

fn windows(value: Json) -> JsonObject {
    JsonObject::from_iter([("access_windows".to_owned(), value)])
}

fn spec(group: &str, id: &str, info: &[(&str, Json)]) -> DeploymentSpec {
    let mut model_info = JsonObject::from_iter([("id".to_owned(), text(id))]);
    for (key, value) in info {
        model_info.insert((*key).to_owned(), value.clone());
    }
    DeploymentSpec::new(
        group.to_owned(),
        JsonObject::new(),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    deployments
        .iter()
        .map(|deployment| deployment.id().unwrap().to_owned())
        .collect()
}

fn admin(team_id: Option<&str>) -> Authority<'_> {
    Authority {
        team_id,
        proxy_admin: true,
    }
}

// A team-owned pool whose deployments answer to one model name, optionally under a team public name too.
fn team_pool(model: &str, public_name: &str, owners: &[(&str, &str)]) -> Vec<DeploymentSpec> {
    owners
        .iter()
        .map(|&(id, team)| {
            spec(
                model,
                id,
                &[
                    ("team_id", text(team)),
                    ("team_public_model_name", text(public_name)),
                ],
            )
        })
        .collect()
}

const AMBIGUOUS_TEAMS_MESSAGE: &str = "Model name 'shared' matches deployments from multiple teams. Specify the deployment ID directly to disambiguate.";

fn absent_and_null_parse_empty() {
    let zones = database();
    assert!(
        parse_access_windows(&JsonObject::new(), &zones)
            .unwrap()
            .is_empty()
    );
    assert!(
        parse_access_windows(&windows(Json::Null), &zones)
            .unwrap()
            .is_empty()
    );
    assert!(
        parse_access_windows(&windows(Json::List(Vec::new())), &zones)
            .unwrap()
            .is_empty()
    );
}

fn parses_a_window_with_its_zone_and_teams() {
    let model_info = windows(Json::List(vec![window(
        "09:00",
        "17:00",
        "Europe/Berlin",
        &["t1", "t2"],
    )]));
    let parsed = parse_access_windows(&model_info, &database()).unwrap();
    assert_eq!(parsed.len(), 1);
    let first = parsed.first().unwrap();
    assert_eq!(first.timezone, "Europe/Berlin");
    assert_eq!(first.team_ids, ["t1", "t2"]);
    assert_eq!(first.zone.iana_name(), Some("Europe/Berlin"));
}

fn config_error_is_none_for_valid_values() {
    let zones = database();
    assert!(access_windows_config_error(&JsonObject::new(), "gpt", &zones).is_none());
    assert!(access_windows_config_error(&windows(Json::Null), "gpt", &zones).is_none());
    assert!(
        access_windows_config_error(
            &windows(Json::List(vec![window("22:00", "06:00", "UTC", &["t"])])),
            "gpt",
            &zones
        )
        .is_none()
    );
}

fn config_error_names_the_first_invalid_field() {
    let zones = database();
    let error = access_windows_config_error(
        &windows(Json::List(vec![window("9:00", "17:00", "UTC", &["t"])])),
        "gpt",
        &zones,
    )
    .unwrap();
    assert_eq!(
        error,
        "model 'gpt': invalid model_info.access_windows: 0.start: must be a time like 09:00"
    );
}

fn an_active_window_keeps_the_calling_team() {
    let deployments = [spec(
        "gpt",
        "d",
        &[(
            "access_windows",
            Json::List(vec![window("09:00", "17:00", "UTC", &["mine"])]),
        )],
    )];
    let kept = filter_reserved_deployments(
        &deployments,
        Some("mine"),
        at("2026-01-05T10:00:00Z"),
        &database(),
    )
    .unwrap();
    assert_eq!(ids(&kept.deployments), ["d"]);
    assert!(kept.blocking_window.is_none());
}

fn a_window_blocks_other_teams_only_while_active() {
    let deployments = [spec(
        "gpt",
        "d",
        &[(
            "access_windows",
            Json::List(vec![window("09:00", "17:00", "UTC", &["mine"])]),
        )],
    )];
    let blocked = filter_reserved_deployments(
        &deployments,
        Some("other"),
        at("2026-01-05T10:00:00Z"),
        &database(),
    )
    .unwrap();
    assert!(blocked.deployments.is_empty());
    assert_eq!(
        blocked.blocking_window.map(|window| window.timezone),
        Some(String::from("UTC"))
    );
    let kept = filter_reserved_deployments(
        &deployments,
        Some("other"),
        at("2026-01-05T18:00:00Z"),
        &database(),
    )
    .unwrap();
    assert_eq!(ids(&kept.deployments), ["d"]);
}

fn a_shared_deployment_survives_any_team() {
    let deployments = [spec("gpt", "a", &[]), spec("gpt", "b", &[])];
    let kept = filter_team_based_models(
        deployments.to_vec(),
        Authority {
            team_id: Some("t1"),
            proxy_admin: false,
        },
        "gpt",
    )
    .unwrap();
    assert_eq!(ids(&kept), ["a", "b"]);
}

fn the_team_filter_drops_other_teams() {
    let deployments = [
        spec("gpt", "a", &[]),
        spec("gpt", "b", &[("team_id", text("t1"))]),
        spec("gpt", "c", &[("team_id", text("t2"))]),
    ];
    let kept = filter_team_based_models(
        deployments.to_vec(),
        Authority {
            team_id: Some("t1"),
            proxy_admin: false,
        },
        "gpt",
    )
    .unwrap();
    assert_eq!(ids(&kept), ["a", "b"]);
}

fn an_admin_sees_one_team_pool_unchanged() {
    let deployments = [
        spec("gpt", "a", &[("team_id", text("t1"))]),
        spec("gpt", "b", &[("team_id", text("t1"))]),
    ];
    let kept = filter_team_based_models(
        deployments.to_vec(),
        Authority {
            team_id: None,
            proxy_admin: true,
        },
        "gpt",
    )
    .unwrap();
    assert_eq!(ids(&kept), ["a", "b"]);
}

fn a_non_admin_without_a_team_sees_only_shared_deployments() {
    let deployments = [
        spec("gpt", "a", &[]),
        spec("gpt", "b", &[("team_id", text("t1"))]),
    ];
    let kept = filter_team_based_models(deployments.to_vec(), Authority::default(), "gpt").unwrap();
    assert_eq!(ids(&kept), ["a"]);
}

// The admin arm needs proxy_admin: a caller with no team and no admin right never keeps another team's pool.
fn a_non_admin_without_a_team_sees_no_team_pool() {
    let deployments = [
        spec("gpt", "a", &[("team_id", text("t1"))]),
        spec("gpt", "b", &[("team_id", text("t1"))]),
    ];
    let kept = filter_team_based_models(deployments.to_vec(), Authority::default(), "gpt").unwrap();
    assert!(kept.is_empty());
}

fn an_empty_team_id_counts_as_no_team() {
    let deployments = [
        spec("gpt", "a", &[]),
        spec("gpt", "b", &[("team_id", text("t1"))]),
    ];
    let kept = filter_team_based_models(
        deployments.to_vec(),
        Authority {
            team_id: Some(""),
            proxy_admin: false,
        },
        "gpt",
    )
    .unwrap();
    assert_eq!(ids(&kept), ["a"]);
    let blank_owner = [
        spec("gpt", "blank", &[("team_id", text(""))]),
        spec("gpt", "owned", &[("team_id", text("t1"))]),
    ];
    let kept_blank = filter_team_based_models(
        blank_owner.to_vec(),
        Authority {
            team_id: Some(""),
            proxy_admin: false,
        },
        "gpt",
    )
    .unwrap();
    assert!(kept_blank.is_empty());
}

// a same-day window is start-inclusive and end-exclusive, to the millisecond
fn same_day_bounds_are_half_open() {
    let parsed = parse_access_windows(
        &windows(Json::List(vec![window("09:00", "17:00", "UTC", &["t"])])),
        &database(),
    )
    .unwrap();
    let first = parsed.first().unwrap();
    for (instant, active) in [
        ("2026-01-05T08:59:59.999Z", false),
        ("2026-01-05T09:00:00Z", true),
        ("2026-01-05T16:59:59.999Z", true),
        ("2026-01-05T17:00:00Z", false),
    ] {
        assert_eq!(
            is_window_active(first, at(instant)).unwrap(),
            active,
            "{instant}"
        );
    }
}

fn overnight_bounds_wrap_at_midnight() {
    let parsed = parse_access_windows(
        &windows(Json::List(vec![window("22:00", "06:00", "UTC", &["t"])])),
        &database(),
    )
    .unwrap();
    let first = parsed.first().unwrap();
    for (instant, active) in [
        ("2026-01-05T21:59:59.999Z", false),
        ("2026-01-05T22:00:00Z", true),
        ("2026-01-06T00:00:00Z", true),
        ("2026-01-06T05:59:59.999Z", true),
        ("2026-01-06T06:00:00Z", false),
        ("2026-01-06T12:00:00Z", false),
    ] {
        assert_eq!(
            is_window_active(first, at(instant)).unwrap(),
            active,
            "{instant}"
        );
    }
}

// The parser refuses equal bounds, so this window can only be built by hand, and it wraps the whole day.
fn equal_bounds_wrap_the_whole_day() {
    let equal = ModelAccessWindow {
        start: Time::new(9, 0, 0, 0).unwrap(),
        end: Time::new(9, 0, 0, 0).unwrap(),
        timezone: String::from("UTC"),
        zone: database().get("UTC").unwrap(),
        team_ids: vec![String::from("t")],
    };
    for instant in [
        "2026-01-05T08:59:59.999Z",
        "2026-01-05T09:00:00Z",
        "2026-01-05T23:00:00Z",
    ] {
        assert!(is_window_active(&equal, at(instant)).unwrap(), "{instant}");
    }
}

fn a_dst_change_shifts_the_local_window() {
    let parsed = parse_access_windows(
        &windows(Json::List(vec![window(
            "09:00",
            "17:00",
            "Europe/Berlin",
            &["t"],
        )])),
        &database(),
    )
    .unwrap();
    let first = parsed.first().unwrap();
    // 07:30 UTC is 09:30 in Berlin under summer time and 08:30 under winter time
    assert!(is_window_active(first, at("2026-07-06T07:30:00Z")).unwrap());
    assert!(!is_window_active(first, at("2026-01-05T07:30:00Z")).unwrap());
    // 2026-03-29 01:00 UTC springs forward: 00:30 UTC is 01:30 CET and 01:30 UTC is 03:30 CEST
    let spring = parse_access_windows(
        &windows(Json::List(vec![window(
            "01:00",
            "03:00",
            "Europe/Berlin",
            &["t"],
        )])),
        &database(),
    )
    .unwrap();
    let shifted = spring.first().unwrap();
    assert!(is_window_active(shifted, at("2026-03-29T00:30:00Z")).unwrap());
    assert!(!is_window_active(shifted, at("2026-03-29T01:30:00Z")).unwrap());
}

fn wall_clock_takes_seconds_and_fractions() {
    let zones = database();
    for value in [
        "09:00",
        "09:00:30",
        "09:00:30.5",
        "09:00:30.55",
        "09:00:30.555",
        "09:00:30.5555",
        "09:00:30.55555",
        "09:00:30.123456",
    ] {
        let model_info = windows(Json::List(vec![window(value, "17:00", "UTC", &["t"])]));
        assert!(parse_access_windows(&model_info, &zones).is_ok(), "{value}");
    }
}

// pydantic's time parser reads both '.' and ',' as the fractional-second separator.
fn a_comma_fraction_parses_like_the_dotted_form() {
    let zones = database();
    for separator in ['.', ','] {
        let value = format!("09:00:30{separator}5");
        let model_info = windows(Json::List(vec![window(
            value.as_str(),
            "17:00",
            "UTC",
            &["t"],
        )]));
        let parsed = parse_access_windows(&model_info, &zones).unwrap();
        assert_eq!(
            parsed.first().unwrap().start,
            Time::new(9, 0, 30, 500_000_000).unwrap(),
            "{value}"
        );
    }
}

fn wall_clock_refuses_malformed_values_and_offsets() {
    let zones = database();
    for (value, expected) in [
        ("9:00", "0.start: must be a time like 09:00"),
        (
            "09:00:00+00:00",
            "0.start: must be a local wall-clock time without a UTC offset",
        ),
        (
            "09:00:00Z",
            "0.start: must be a local wall-clock time without a UTC offset",
        ),
        ("24:00", "0.start: must be a time like 09:00"),
        ("09:60", "0.start: must be a time like 09:00"),
        ("0::00", "0.start: must be a time like 09:00"),
        ("09:00:00.12a", "0.start: must be a time like 09:00"),
        ("09:00:00.1234567", "0.start: must be a time like 09:00"),
    ] {
        let model_info = windows(Json::List(vec![window(value, "17:00", "UTC", &["t"])]));
        let error = parse_access_windows(&model_info, &zones).unwrap_err();
        assert_eq!(error.to_string(), expected, "{value}");
    }
}

fn a_zone_name_is_matched_case_sensitively() {
    let zones = database();
    for accepted in ["UTC", "Europe/Berlin", "America/New_York"] {
        let model_info = windows(Json::List(vec![window("09:00", "17:00", accepted, &["t"])]));
        assert!(
            parse_access_windows(&model_info, &zones).is_ok(),
            "{accepted}"
        );
    }
    for rejected in ["utc", "europe/berlin", "Not/AZone"] {
        let model_info = windows(Json::List(vec![window("09:00", "17:00", rejected, &["t"])]));
        let error = parse_access_windows(&model_info, &zones).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("0.timezone: unknown IANA timezone '{rejected}'")
        );
    }
}

fn team_ids_must_hold_non_empty_entries() {
    let zones = database();
    let empty = windows(Json::List(vec![window("09:00", "17:00", "UTC", &[])]));
    assert_eq!(
        parse_access_windows(&empty, &zones)
            .unwrap_err()
            .to_string(),
        "0.team_ids: must be a non-empty list of team ids"
    );
    let blank = windows(Json::List(vec![window("09:00", "17:00", "UTC", &[""])]));
    assert_eq!(
        parse_access_windows(&blank, &zones)
            .unwrap_err()
            .to_string(),
        "0.team_ids: entries must be non-empty"
    );
    let number = windows(Json::List(vec![Json::Object(JsonObject::from_iter([
        ("start".to_owned(), text("09:00")),
        ("end".to_owned(), text("17:00")),
        ("timezone".to_owned(), text("UTC")),
        ("team_ids".to_owned(), Json::List(vec![Json::Bool(true)])),
    ]))]));
    assert_eq!(
        parse_access_windows(&number, &zones)
            .unwrap_err()
            .to_string(),
        "0.team_ids.0: must hold non-empty text team ids"
    );
}

fn equal_bounds_and_extra_fields_are_refused() {
    let zones = database();
    let equal = windows(Json::List(vec![window("09:00", "09:00", "UTC", &["t"])]));
    assert_eq!(
        parse_access_windows(&equal, &zones)
            .unwrap_err()
            .to_string(),
        "0: start and end must differ"
    );
    let extra = windows(Json::List(vec![Json::Object(JsonObject::from_iter([
        ("start".to_owned(), text("09:00")),
        ("end".to_owned(), text("17:00")),
        ("timezone".to_owned(), text("UTC")),
        ("team_ids".to_owned(), Json::List(vec![text("t")])),
        ("colour".to_owned(), text("blue")),
    ]))]));
    assert_eq!(
        parse_access_windows(&extra, &zones)
            .unwrap_err()
            .to_string(),
        "0.colour: is not a window field"
    );
}

fn a_non_list_value_is_refused() {
    let error =
        parse_access_windows(&windows(Json::Text(String::from("09:00"))), &database()).unwrap_err();
    assert_eq!(
        error.to_string(),
        ": access_windows must be a list of windows"
    );
}

fn a_non_object_window_is_refused() {
    let entries = windows(Json::List(vec![Json::Text(String::from("09:00"))]));
    let error = parse_access_windows(&entries, &database()).unwrap_err();
    assert_eq!(error.to_string(), "0: must be a window object");
}

fn a_missing_or_typed_window_field_is_refused() {
    let missing = JsonObject::from_iter([
        ("end".to_owned(), text("17:00")),
        ("timezone".to_owned(), text("UTC")),
        ("team_ids".to_owned(), Json::List(vec![text("t")])),
    ]);
    let typed = JsonObject::from_iter([
        ("start".to_owned(), text("09:00")),
        ("end".to_owned(), text("17:00")),
        ("timezone".to_owned(), Json::Bool(true)),
        ("team_ids".to_owned(), Json::List(vec![text("t")])),
    ]);
    for (entries, expected) in [
        (missing, "0.start: is required"),
        (typed, "0.timezone: must be an IANA timezone name"),
    ] {
        let value = windows(Json::List(vec![Json::Object(entries)]));
        let error = parse_access_windows(&value, &database()).unwrap_err();
        assert_eq!(error.to_string(), expected);
    }
}

fn a_blocked_pool_is_refused_with_the_window_end() {
    let deployments = vec![spec(
        "gpt",
        "d",
        &[(
            "access_windows",
            Json::List(vec![window("22:00", "06:00", "UTC", &["mine"])]),
        )],
    )];
    let error = reserved_deployments(
        "gpt",
        &deployments,
        Some("other"),
        at("2026-01-05T23:00:00Z"),
        &database(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Deployment gpt is reserved for another team until 06:00 UTC"
    );
}

// model_info.team_id: null reads as no owner, so the admin arm leaves it out and the shared deployment stays.
fn an_admin_pool_with_a_null_owner_is_only_filtered() {
    let deployments = [
        spec("shared", "shared", &[("team_id", Json::Null)]),
        spec("shared", "owned", &[("team_id", text("t1"))]),
    ];
    let kept = filter_team_based_models(
        deployments.to_vec(),
        Authority {
            team_id: None,
            proxy_admin: true,
        },
        "shared",
    )
    .unwrap();
    assert_eq!(ids(&kept), ["shared"]);
}

fn an_invalid_window_is_refused_with_its_deployment() {
    let deployments = vec![spec(
        "gpt",
        "d",
        &[(
            "access_windows",
            Json::List(vec![Json::Object(JsonObject::from_iter([
                ("start".to_owned(), text("09:00")),
                ("end".to_owned(), text("09:00")),
                ("timezone".to_owned(), text("UTC")),
                ("team_ids".to_owned(), Json::List(vec![text("t")])),
            ]))]),
        )],
    )];
    let error = reserved_deployments(
        "gpt",
        &deployments,
        Some("other"),
        at("2026-01-05T10:00:00Z"),
        &database(),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CatalogLookupError::Reserved { message } if message.ends_with("0: start and end must differ")),
        "{error}"
    );
}

fn an_admin_pool_named_by_two_teams_is_refused() {
    let pool = team_pool("shared", "shared", &[("a", "t1"), ("b", "t2")]);
    let error = filter_team_based_models(pool, admin(None), "shared").unwrap_err();
    assert_eq!(
        error,
        CatalogLookupError::AmbiguousTeams {
            model: String::from("shared")
        }
    );
    assert_eq!(error.to_string(), AMBIGUOUS_TEAMS_MESSAGE);
}

fn kept_deployments_keep_catalog_order() {
    let deployments = vec![
        spec("gpt", "a", &[]),
        spec("gpt", "b", &[("team_id", text("t1"))]),
        spec("gpt", "c", &[("team_id", text("t2"))]),
        spec("gpt", "d", &[]),
    ];
    let kept = filter_team_based_models(
        deployments,
        Authority {
            team_id: Some("t1"),
            proxy_admin: false,
        },
        "gpt",
    )
    .unwrap();
    assert_eq!(ids(&kept), ["a", "b", "d"]);
}

fn a_pool_with_one_blocked_deployment_keeps_the_rest() {
    let deployments = vec![
        spec(
            "gpt",
            "blocked",
            &[(
                "access_windows",
                Json::List(vec![window("09:00", "17:00", "UTC", &["mine"])]),
            )],
        ),
        spec("gpt", "open", &[]),
    ];
    let kept = reserved_deployments(
        "gpt",
        &deployments,
        Some("other"),
        at("2026-01-05T10:00:00Z"),
        &database(),
    )
    .unwrap();
    assert_eq!(ids(&kept), ["open"]);
}

fn an_admin_pool_keeps_only_authorized_teams() {
    let pool = team_pool("shared", "shared", &[("a", "t1"), ("b", "t2")]);
    let kept = filter_team_based_models(pool, admin(Some("t1")), "shared").unwrap();
    assert_eq!(ids(&kept), ["a"]);
}

fn an_admin_pool_without_a_named_team_keeps_only_shared() {
    let mut pool = team_pool("shared", "shared", &[("c", "t1")]);
    pool.push(spec("shared", "open", &[]));
    let kept = filter_team_based_models(pool, admin(None), "shared").unwrap();
    assert_eq!(ids(&kept), ["open"]);
}

// LiteLLM's admin arm also matches a team's public model name, not only the deployment's own model name.
fn an_admin_pool_named_by_its_public_name_is_kept() {
    let pool = team_pool("internal", "public", &[("a", "t1"), ("b", "t1")]);
    let kept = filter_team_based_models(pool, admin(None), "public").unwrap();
    assert_eq!(ids(&kept), ["a", "b"]);
}

fn an_admin_pool_unnamed_by_the_request_is_only_filtered() {
    let deployments = vec![
        spec("other", "a", &[("team_id", text("t1"))]),
        spec("other", "b", &[("team_id", text("t2"))]),
    ];
    let kept = filter_team_based_models(
        deployments,
        Authority {
            team_id: None,
            proxy_admin: true,
        },
        "shared",
    )
    .unwrap();
    assert!(kept.is_empty());
}

#[rstest]
#[case::absent_and_null_parse_empty(absent_and_null_parse_empty)]
#[case::parses_a_window_with_its_zone_and_teams(parses_a_window_with_its_zone_and_teams)]
#[case::config_error_is_none_for_valid_values(config_error_is_none_for_valid_values)]
#[case::config_error_names_the_first_invalid_field(config_error_names_the_first_invalid_field)]
#[case::an_active_window_keeps_the_calling_team(an_active_window_keeps_the_calling_team)]
#[case::a_window_blocks_other_teams_only_while_active(
    a_window_blocks_other_teams_only_while_active
)]
#[case::a_shared_deployment_survives_any_team(a_shared_deployment_survives_any_team)]
#[case::the_team_filter_drops_other_teams(the_team_filter_drops_other_teams)]
#[case::an_admin_sees_one_team_pool_unchanged(an_admin_sees_one_team_pool_unchanged)]
#[case::a_non_admin_without_a_team_sees_only_shared_deployments(
    a_non_admin_without_a_team_sees_only_shared_deployments
)]
#[case::a_non_admin_without_a_team_sees_no_team_pool(a_non_admin_without_a_team_sees_no_team_pool)]
#[case::an_empty_team_id_counts_as_no_team(an_empty_team_id_counts_as_no_team)]
#[case::an_admin_pool_keeps_only_authorized_teams(an_admin_pool_keeps_only_authorized_teams)]
#[case::an_admin_pool_unnamed_by_the_request_is_only_filtered(
    an_admin_pool_unnamed_by_the_request_is_only_filtered
)]
#[case::an_admin_pool_without_a_named_team_keeps_only_shared(
    an_admin_pool_without_a_named_team_keeps_only_shared
)]
#[case::an_admin_pool_named_by_its_public_name_is_kept(
    an_admin_pool_named_by_its_public_name_is_kept
)]
#[case::a_comma_fraction_parses_like_the_dotted_form(a_comma_fraction_parses_like_the_dotted_form)]
fn access_windows_behaviors(#[case] run: fn()) {
    run();
}

#[rstest]
#[case::same_day_bounds_are_half_open(same_day_bounds_are_half_open)]
#[case::overnight_bounds_wrap_at_midnight(overnight_bounds_wrap_at_midnight)]
#[case::equal_bounds_wrap_the_whole_day(equal_bounds_wrap_the_whole_day)]
#[case::a_dst_change_shifts_the_local_window(a_dst_change_shifts_the_local_window)]
#[case::wall_clock_takes_seconds_and_fractions(wall_clock_takes_seconds_and_fractions)]
#[case::wall_clock_refuses_malformed_values_and_offsets(
    wall_clock_refuses_malformed_values_and_offsets
)]
#[case::a_zone_name_is_matched_case_sensitively(a_zone_name_is_matched_case_sensitively)]
#[case::team_ids_must_hold_non_empty_entries(team_ids_must_hold_non_empty_entries)]
#[case::equal_bounds_and_extra_fields_are_refused(equal_bounds_and_extra_fields_are_refused)]
#[case::a_non_list_value_is_refused(a_non_list_value_is_refused)]
#[case::a_non_object_window_is_refused(a_non_object_window_is_refused)]
#[case::a_missing_or_typed_window_field_is_refused(a_missing_or_typed_window_field_is_refused)]
#[case::a_blocked_pool_is_refused_with_the_window_end(
    a_blocked_pool_is_refused_with_the_window_end
)]
#[case::an_invalid_window_is_refused_with_its_deployment(
    an_invalid_window_is_refused_with_its_deployment
)]
#[case::a_pool_with_one_blocked_deployment_keeps_the_rest(
    a_pool_with_one_blocked_deployment_keeps_the_rest
)]
#[case::an_admin_pool_with_a_null_owner_is_only_filtered(
    an_admin_pool_with_a_null_owner_is_only_filtered
)]
#[case::an_admin_pool_named_by_two_teams_is_refused(an_admin_pool_named_by_two_teams_is_refused)]
#[case::kept_deployments_keep_catalog_order(kept_deployments_keep_catalog_order)]
fn access_windows_boundaries(#[case] run: fn()) {
    run();
}
