#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::PillarError;
use super::error_fallback::handle_api_error;
use super::settings::{FallbackAction, OnFlaggedAction, PillarSettings};
use crate::guardrail_base::failure::GuardrailFailure;

fn pillar_settings(fallback_on_error: FallbackAction) -> PillarSettings {
    PillarSettings {
        api_key: None,
        api_base: String::from("https://pillar.test"),
        on_flagged_action: OnFlaggedAction::Monitor,
        async_mode: None,
        persist_session: None,
        include_scanners: true,
        include_evidence: true,
        fallback_on_error,
        timeout: std::time::Duration::from_secs(5),
    }
}

fn request_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([(
        "messages".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::new())]),
    )])
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

#[test]
fn allow_returns_the_request_data_unchanged() {
    let data = request_data();
    let returned = handle_api_error(
        &PillarError::Status { status: 500 },
        data.clone(),
        &pillar_settings(FallbackAction::Allow),
    )
    .expect("allow does not fail");
    assert_eq!(returned, data);
}

#[test]
fn block_raises_the_503_detail() {
    let error = handle_api_error(
        &PillarError::Status { status: 500 },
        request_data(),
        &pillar_settings(FallbackAction::Block),
    )
    .expect_err("block fails the request");
    let PillarError::Http(GuardrailFailure::HttpObject { status, detail }) = error else {
        panic!("the fallback block is an HTTP 503");
    };
    assert_eq!(status, 503);
    assert_eq!(
        detail.get("error"),
        Some(&text("Pillar Security Guardrail Unavailable"))
    );
    assert_eq!(
        detail.get("message"),
        Some(&text(
            "Security scanning service is temporarily unavailable and fallback is set to block"
        ))
    );
    assert!(detail.contains_key("original_error"));
}

#[test]
fn the_original_error_carries_the_betterlite_message() {
    let failure = PillarError::Status { status: 500 };
    let error = handle_api_error(
        &failure,
        request_data(),
        &pillar_settings(FallbackAction::Block),
    )
    .expect_err("block fails the request");
    let PillarError::Http(GuardrailFailure::HttpObject { detail, .. }) = error else {
        panic!("the fallback block is an HTTP 503");
    };
    assert_eq!(
        detail.get("original_error"),
        Some(&text("the Pillar API answered with status 500"))
    );
}
