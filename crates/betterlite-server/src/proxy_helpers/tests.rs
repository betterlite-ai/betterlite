#![cfg(test)]

use std::collections::BTreeMap;
use std::io::Error as IoError;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use chrono::NaiveDate;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::checks::{
    Detail, ErrorText, HttpException, NOT_PREMIUM_USER, ProxyCheckError, check_team_models,
    error_message_str, premium_user_check, prisma_client_or_throw,
};
use super::projected::{
    ProjectionError, is_projected_spend_over_limit, month_end_date, projected_spend_over_limit,
};

fn json_map(fields: &[(&str, Json)]) -> BTreeMap<String, Json> {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
}

fn decimal(value: &str) -> Decimal {
    value.parse().unwrap()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn a_text_detail_is_the_error_message() {
    let error = HttpException {
        status: StatusCode::BAD_REQUEST,
        detail: Some(Detail::Text("something went wrong".to_owned())),
        message: None,
    };
    assert_eq!(
        error_message_str(ErrorText::Http(&error)),
        "something went wrong"
    );
}

#[test]
fn a_map_detail_is_standard_json() {
    let detail = json_map(&[
        ("error", text("bad input")),
        ("code", text("invalid_request")),
    ]);
    let error = HttpException {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        detail: Some(Detail::Map(detail)),
        message: None,
    };
    assert_eq!(
        error_message_str(ErrorText::Http(&error)),
        r#"{"code":"invalid_request","error":"bad input"}"#
    );
}

#[test]
fn a_message_attribute_is_read_when_the_detail_is_not_text_or_a_map() {
    let error = HttpException {
        status: StatusCode::BAD_REQUEST,
        detail: None,
        message: Some(Detail::Text("labelled".to_owned())),
    };
    assert_eq!(error_message_str(ErrorText::Http(&error)), "labelled");
}

#[test]
fn a_map_message_attribute_is_standard_json() {
    let error = HttpException {
        status: StatusCode::BAD_REQUEST,
        detail: None,
        message: Some(Detail::Map(json_map(&[("error", text("labelled"))]))),
    };
    assert_eq!(
        error_message_str(ErrorText::Http(&error)),
        r#"{"error":"labelled"}"#
    );
}

#[test]
fn another_error_gives_its_display_text() {
    let error = IoError::other("boom");
    assert_eq!(error_message_str(ErrorText::Other(&error)), "boom");
}

#[test]
fn an_invalid_team_model_is_rejected_with_the_pinned_message() {
    let mut team_config = json_map(&[
        (
            "models",
            Json::List(vec![text("gpt-4o"), text("gpt-4o-mini")]),
        ),
        ("max_budget", Json::Number(Number::from(100_u64))),
    ]);
    let request_data = json_map(&[("model", text("claude-haiku"))]);
    let error =
        check_team_models(Some("team-1"), Some(&mut team_config), Some(&request_data)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid model for team team-1: claude-haiku.  Valid models for team are: gpt-4o, gpt-4o-mini\n"
    );
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!team_config.contains_key("models"));
}

#[test]
fn an_allowed_model_still_removes_the_models_key() {
    let mut team_config = json_map(&[("models", Json::List(vec![text("gpt-4o")]))]);
    let request_data = json_map(&[("model", text("gpt-4o"))]);
    check_team_models(Some("team-1"), Some(&mut team_config), Some(&request_data)).unwrap();
    assert!(team_config.is_empty());
}

#[test]
fn a_team_config_without_models_is_a_no_op() {
    let mut team_config = json_map(&[("max_budget", Json::Number(Number::from(100_u64)))]);
    let request_data = json_map(&[("model", text("anything"))]);
    check_team_models(Some("team-1"), Some(&mut team_config), Some(&request_data)).unwrap();
    assert_eq!(team_config.len(), 1);
}

#[test]
fn a_missing_team_id_short_circuits_before_the_config_changes() {
    let mut team_config = json_map(&[("models", Json::List(vec![text("only-this")]))]);
    let request_data = json_map(&[("model", text("anything-else"))]);
    check_team_models(None, Some(&mut team_config), Some(&request_data)).unwrap();
    assert!(team_config.contains_key("models"));
}

#[test]
fn missing_request_data_short_circuits_before_the_config_changes() {
    let mut team_config = json_map(&[("models", Json::List(vec![text("only-this")]))]);
    check_team_models(Some("team-1"), Some(&mut team_config), None).unwrap();
    assert!(team_config.contains_key("models"));
}

#[test]
fn models_that_are_not_text_are_rejected() {
    let mut team_config = json_map(&[(
        "models",
        Json::List(vec![Json::Number(Number::from(4_u64))]),
    )]);
    let request_data = json_map(&[("model", text("gpt-4o"))]);
    let error =
        check_team_models(Some("team-1"), Some(&mut team_config), Some(&request_data)).unwrap_err();
    assert_eq!(error, ProxyCheckError::InvalidTeamModels);
}

#[test]
fn request_data_needs_a_model_name() {
    let mut team_config = json_map(&[("models", Json::List(vec![text("gpt-4o")]))]);
    let request_data = json_map(&[]);
    let error =
        check_team_models(Some("team-1"), Some(&mut team_config), Some(&request_data)).unwrap_err();
    assert_eq!(error, ProxyCheckError::MissingRequestModel);
}

#[test]
fn a_premium_user_passes_the_check() {
    assert_eq!(premium_user_check(true, Some("sso")), Ok(()));
}

#[test]
fn a_feature_gate_names_the_feature_and_is_forbidden() {
    let error = premium_user_check(false, Some("sso")).unwrap_err();
    let message =
        format!("This feature is only available for enterprise users: sso. {NOT_PREMIUM_USER}");
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(error.to_string(), message);
    assert_eq!(error.detail(), Some(json_map(&[("error", text(&message))])));
}

#[test]
fn a_gate_without_a_feature_omits_the_feature_name() {
    let error = premium_user_check(false, None).unwrap_err();
    let message =
        format!("This feature is only available for enterprise users. {NOT_PREMIUM_USER}");
    assert_eq!(error.to_string(), message);
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
}

#[test]
fn a_database_handle_passes_through() {
    let database = 7_u8;
    assert_eq!(
        prisma_client_or_throw(Some(&database), "Database not available"),
        Ok(&database)
    );
}

#[test]
fn a_missing_database_is_a_500_with_the_callers_message() {
    let error = prisma_client_or_throw::<u8>(None, "Database not available").unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error.to_string(), "Database not available");
    assert_eq!(
        error.detail(),
        Some(json_map(&[("error", text("Database not available"))]))
    );
}

#[rstest]
#[case::january("2024-01-15", "2024-01-31")]
#[case::june("2024-06-15", "2024-06-30")]
#[case::leap_february("2024-02-10", "2024-02-29")]
#[case::non_leap_february("2023-02-10", "2023-02-28")]
#[case::december("2024-12-20", "2024-12-31")]
fn the_month_ends_on_its_last_day(#[case] today: &str, #[case] expected: &str) {
    assert_eq!(month_end_date(date(today)), date(expected));
}

#[test]
fn the_first_day_uses_the_whole_spend_as_the_daily_rate() {
    assert_eq!(
        is_projected_spend_over_limit(decimal("100"), Some(decimal("150")), date("2024-06-01")),
        Ok(true)
    );
    assert_eq!(
        is_projected_spend_over_limit(decimal("100"), Some(decimal("5000")), date("2024-06-01")),
        Ok(false)
    );
}

#[test]
fn projection_without_a_limit_never_exceeds_it() {
    assert_eq!(
        is_projected_spend_over_limit(decimal("1000"), None, date("2024-06-15")),
        Ok(false)
    );
    assert_eq!(
        projected_spend_over_limit(decimal("1000"), None, date("2024-06-15")),
        Ok(None)
    );
}

#[rstest]
#[case::over("140", "200", "2024-06-15", Some(("290", "2024-06-21")))]
#[case::under("140", "300", "2024-06-15", None)]
#[case::exactly_the_limit("140", "290", "2024-06-15", None)]
fn the_limit_date_adds_the_remaining_budget_over_the_daily_rate(
    #[case] spend: &str,
    #[case] soft_limit: &str,
    #[case] today: &str,
    #[case] expected: Option<(&str, &str)>,
) {
    let reached =
        projected_spend_over_limit(decimal(spend), Some(decimal(soft_limit)), date(today)).unwrap();
    match expected {
        Some((projected, day)) => {
            let limit = reached.unwrap();
            assert_eq!(limit.projected_spend, decimal(projected));
            assert_eq!(limit.limit_exceed_date, date(day));
        }
        None => assert!(reached.is_none()),
    }
}

#[test]
fn the_first_day_reaches_the_limit_today() {
    let limit =
        projected_spend_over_limit(decimal("100"), Some(decimal("110")), date("2024-06-01"))
            .unwrap()
            .unwrap();
    assert_eq!(limit.limit_exceed_date, date("2024-06-01"));
}

#[test]
fn december_projects_across_the_year_end() {
    let limit =
        projected_spend_over_limit(decimal("190"), Some(decimal("290")), date("2024-12-20"))
            .unwrap()
            .unwrap();
    assert_eq!(limit.projected_spend, decimal("300"));
    assert_eq!(limit.limit_exceed_date, date("2024-12-30"));
}

#[test]
fn a_spend_already_over_the_limit_reaches_it_today() {
    let limit =
        projected_spend_over_limit(decimal("200"), Some(decimal("150")), date("2024-06-15"))
            .unwrap()
            .unwrap();
    assert_eq!(limit.limit_exceed_date, date("2024-06-15"));
}

#[test]
fn a_zero_daily_rate_reaches_the_limit_today() {
    let limit = projected_spend_over_limit(Decimal::ZERO, Some(decimal("-5")), date("2024-06-15"))
        .unwrap()
        .unwrap();
    assert_eq!(limit.limit_exceed_date, date("2024-06-15"));
    assert_eq!(limit.projected_spend, Decimal::ZERO);
}

#[test]
fn an_overflowing_projection_is_an_error() {
    assert_eq!(
        is_projected_spend_over_limit(Decimal::MAX, Some(Decimal::ONE), date("2024-06-02")),
        Err(ProjectionError::Arithmetic)
    );
    assert_eq!(
        projected_spend_over_limit(Decimal::MAX, Some(Decimal::ONE), date("2024-06-02")),
        Err(ProjectionError::Arithmetic)
    );
}
