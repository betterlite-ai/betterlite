#![cfg(test)]

use serde_json::json;

use super::{ADMIN, bad_request, caller, fixtures, state_with_general, update};
use crate::config_fields::error::ConfigFieldError;
use crate::config_fields::http::decode_body;
use crate::config_fields::ports::ConfigFieldServices;
use crate::config_fields::records::{ConfigFieldUpdate, ConfigFieldUpdateResponse};
use crate::config_fields::write::update_config_general_settings;

#[tokio::test]
async fn update_writes_a_declared_alert_types_list() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let written = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update("alert_types", json!(["budget_alerts", "llm_exceptions"])),
    )
    .await
    .expect("an alert_types list");
    match written {
        ConfigFieldUpdateResponse::Row(row) => {
            assert_eq!(
                row.param_value["alert_types"],
                json!(["budget_alerts", "llm_exceptions"])
            );
        }
        ConfigFieldUpdateResponse::Ui(body) => panic!("a row write, got {body:?}"),
    }
    assert_eq!(fakes.store.inserted_rows().len(), 1);
}

#[tokio::test]
async fn update_refuses_a_bare_alert_types_string() {
    let fakes = fixtures();
    let mut state = state_with_general("{}");
    let refusal = update_config_general_settings(
        services!(fakes, state),
        &caller(ADMIN),
        &update("alert_types", json!("budget_alerts")),
    )
    .await
    .expect_err("a bare alert type");
    assert_eq!(
        bad_request(refusal),
        "Invalid type of field value=<class 'str'> passed in."
    );
    assert!(fakes.store.inserted_rows().is_empty());
}

#[test]
fn decode_body_refuses_a_malformed_update_body() {
    let refusal =
        decode_body::<ConfigFieldUpdate>(br#"{"field_name": "#).expect_err("a malformed body");
    assert!(matches!(
        refusal,
        ConfigFieldError::Unprocessable {
            detail: "Invalid request body"
        }
    ));
}
