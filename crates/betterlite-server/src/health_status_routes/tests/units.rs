use std::ptr::eq;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use crate::health_status_routes::convert::{convert_health_check, isoformat};
use crate::health_status_routes::prisma::check_prisma_client;
use crate::health_status_routes::target::resolve_target_model_name;
use crate::health_status_routes::tests::support::{
    capture_logs, deployment, failing_router, instant, instant_with_fraction, object, render,
    router, row, text, text_field, unconnected,
};

#[test]
fn convert_returns_the_twelve_keys_in_litellm_order() {
    let stored = row("hc-1", "group-a", instant("2026-03-15 10:00:00"));
    let converted = convert_health_check(&stored);
    let keys: Vec<&str> = converted.keys().map(String::as_str).collect();
    let expected = concat!(
        "health_check_id,model_name,model_id,status,healthy_count,unhealthy_count,",
        "error_message,response_time_ms,details,checked_by,checked_at,created_at"
    );
    assert_eq!(keys.join(","), expected);
    assert_eq!(text_field(&converted, "model_id"), Some("id-a".to_owned()));
    let response_time = Number::from_f64(12.5_f64).expect("a finite float");

    assert_eq!(
        converted.get("response_time_ms"),
        Some(&Json::Number(response_time))
    );
    assert_eq!(
        converted.get("health_check_id"),
        Some(&Json::Text("hc-1".to_owned()))
    );
    assert_eq!(
        converted.get("checked_at"),
        Some(&Json::Text("2026-03-15T10:00:00+00:00".to_owned()))
    );
}

#[test]
fn convert_prints_the_fraction_only_when_the_microseconds_are_not_zero() {
    assert_eq!(
        isoformat(instant("2026-03-15 10:00:00")),
        "2026-03-15T10:00:00+00:00"
    );
    assert_eq!(
        isoformat(instant_with_fraction("2026-03-15 10:00:00.123")),
        "2026-03-15T10:00:00.123000+00:00"
    );
    assert_eq!(
        isoformat(instant_with_fraction("2026-03-15 10:00:00.123456")),
        "2026-03-15T10:00:00.123456+00:00"
    );
    let mut stored = row(
        "hc-2",
        "group-a",
        instant_with_fraction("2026-03-15 10:00:00.5"),
    );
    stored.created_at = instant("2026-03-16 00:00:00");
    let converted = convert_health_check(&stored);
    assert_eq!(
        converted.get("checked_at"),
        Some(&Json::Text("2026-03-15T10:00:00.500000+00:00".to_owned()))
    );
    assert_eq!(
        converted.get("created_at"),
        Some(&Json::Text("2026-03-16T00:00:00+00:00".to_owned()))
    );
}

#[test]
fn convert_keeps_nulls_and_passes_details_through() {
    let mut stored = row("hc-3", "group-a", instant("2026-03-15 10:00:00"));
    stored.model_id = None;
    stored.error_message = None;
    stored.response_time_ms = None;
    stored.checked_by = None;
    stored.details = Some(Json::Object(object(vec![("latency", text("fast"))])));
    let converted = convert_health_check(&stored);
    assert_eq!(converted.get("model_id"), Some(&Json::Null));
    assert_eq!(converted.get("error_message"), Some(&Json::Null));
    assert_eq!(converted.get("response_time_ms"), Some(&Json::Null));
    assert_eq!(converted.get("checked_by"), Some(&Json::Null));
    assert_eq!(converted.get("details"), stored.details.as_ref());
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn target_returns_the_model_when_no_model_id_is_given(#[case] model_id: Option<&str>) {
    let lookup = router(vec![deployment("group-a", "id-a", "openai/a")]);
    assert_eq!(
        resolve_target_model_name(Some("group-a"), model_id, Some(lookup.as_ref()))
            .expect("resolves"),
        Some("group-a".to_owned()),
        "{model_id:?}"
    );
    assert_eq!(
        resolve_target_model_name(None, None, None).expect("resolves"),
        None
    );
}

#[test]
fn target_returns_the_models_name_for_a_known_id() {
    let lookup = router(vec![deployment("group-a", "id-a", "openai/a")]);
    assert_eq!(
        resolve_target_model_name(None, Some("id-a"), Some(lookup.as_ref())).expect("resolves"),
        Some("group-a".to_owned())
    );
}

#[tokio::test]
async fn target_unknown_id_is_a_404_even_beside_a_known_model() {
    let lookup = router(vec![deployment("group-a", "id-a", "openai/a")]);
    let error = resolve_target_model_name(Some("group-a"), Some("missing"), Some(lookup.as_ref()))
        .expect_err("the id is unknown");
    let reply = render(error).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail_error(), "Model with ID missing not found");
}

#[tokio::test]
async fn target_without_a_router_is_a_404() {
    let error = resolve_target_model_name(None, Some("id-a"), None).expect_err("no router");
    let reply = render(error).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail_error(), "Model with ID id-a not found");
}

#[tokio::test]
async fn target_failing_lookup_logs_and_is_a_404() {
    let (buffer, _guard) = capture_logs();
    let lookup = failing_router();
    let error = resolve_target_model_name(None, Some("id-a"), Some(lookup.as_ref()))
        .expect_err("the lookup fails");
    let reply = render(error).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail_error(), "Model with ID id-a not found");
    let logs = buffer.text();
    assert!(
        logs.contains("Error getting deployment for model_id id-a:"),
        "{logs}"
    );
}

#[tokio::test]
async fn check_without_a_store_is_database_not_initialized() {
    let Err(error) = check_prisma_client(None) else {
        panic!("no store gives an error");
    };
    let reply = render(error).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.detail_error(), "Database not initialized");
}

#[test]
fn check_with_a_store_returns_it() {
    let rows = unconnected();
    let found = check_prisma_client(Some(&rows)).expect("the store");
    assert!(eq(found, &raw const rows));
}
