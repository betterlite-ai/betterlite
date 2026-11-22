use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_models::model_registration::JsonObject;

use super::super::errors::HttpFailure;
use super::super::reload::raise_if_degraded_serving;
use super::fixtures::json_object;
use super::{http_failure, ids, list, text};
use crate::proxy_helpers::checks::{Detail, HttpException};

fn written_pair(model_id: &str) -> (String, JsonObject) {
    (
        model_id.to_owned(),
        json_object(&[("access_groups", list(&["group-a"]))]),
    )
}

fn reload_message(access_group: &str, gone: &[&str]) -> String {
    let listed: Vec<String> = gone.iter().map(|id| format!("'{id}'")).collect();
    format!(
        "Access group '{access_group}' was saved to the database, but model id(s) [{}] that this pod was serving are no longer live after the reload it triggered. Other pods reload on their own interval. Check server logs for 'Error upserting deployment' for the cause.",
        listed.join(", ")
    )
}

#[test]
fn reload_metadata_only() {
    let live = ids(&["d1"]);
    let written = vec![written_pair("d1")];
    raise_if_degraded_serving(&ids(&["d1"]), &written, "group-a", None, Some(&live))
        .expect("the live model");
    let already_dead = vec![written_pair("d1")];
    raise_if_degraded_serving(&ids(&[]), &already_dead, "group-a", None, Some(&ids(&[])))
        .expect("a row that was already dead");
}

#[test]
fn reload_saved_but_not_live() {
    let written = vec![written_pair("d1")];
    let error =
        raise_if_degraded_serving(&ids(&["d1"]), &written, "group-a", None, Some(&ids(&[])))
            .expect_err("the dropped model");
    let failure = http_failure(error);
    assert_eq!(failure.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(failure.message, reload_message("group-a", &["d1"]));
}

#[test]
fn reload_collateral_and_order() {
    let written = vec![written_pair("d1"), written_pair("d1")];
    let still_desired = ids(&["d1", "d2", "d3"]);
    let live = ids(&["d4"]);
    let error = raise_if_degraded_serving(
        &ids(&["d1", "d2", "d3", "d4"]),
        &written,
        "group-a",
        Some(&still_desired),
        Some(&live),
    )
    .expect_err("the degraded reload");
    let failure = http_failure(error);
    assert_eq!(failure.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        failure.message,
        reload_message("group-a", &["d1", "d2", "d3"])
    );
}

#[test]
fn reload_snapshot_fallback() {
    let written = vec![written_pair("d1")];
    let error = raise_if_degraded_serving(&ids(&["d1"]), &written, "group-a", None, None)
        .expect_err("the missing live set");
    assert_eq!(
        http_failure(error).message,
        reload_message("group-a", &["d1"])
    );
}

#[test]
fn reload_still_desired_filters_collateral() {
    let error = raise_if_degraded_serving(
        &ids(&["d1", "d2"]),
        &[],
        "group-a",
        Some(&ids(&["d2"])),
        Some(&ids(&[])),
    )
    .expect_err("the collateral drop");
    assert_eq!(
        http_failure(error).message,
        reload_message("group-a", &["d2"])
    );
    raise_if_degraded_serving(
        &ids(&["d1"]),
        &[],
        "group-a",
        Some(&ids(&[])),
        Some(&ids(&[])),
    )
    .expect("a drop the database no longer wants");
}

#[test]
fn http_error_detail_shape() {
    let converted = HttpException::from(HttpFailure {
        status: StatusCode::BAD_REQUEST,
        message: "boom".to_owned(),
    });
    assert_eq!(converted.status, StatusCode::BAD_REQUEST);
    assert!(converted.message.is_none());
    match converted.detail {
        Some(Detail::Map(detail)) => {
            assert_eq!(detail, BTreeMap::from([("error".to_owned(), text("boom"))]));
        }
        other => panic!("expected a map detail, got {other:?}"),
    }
}
