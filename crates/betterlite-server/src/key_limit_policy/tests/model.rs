use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::key_limit_policy::error::{Entity, KeyLimitError, Rejection};
use crate::key_limit_policy::limits::{
    MODEL_RPM_FIELD, MODEL_TPM_FIELD, ModelCeilings, check_model_limits, check_rate_limits,
    model_ceilings,
};
use crate::key_limit_policy::request::KeyLimitRequest;
use crate::team_parent_limits::error::RateLimit;

use super::support::{generate, keys, object, rejected, text_detail};

fn ceilings(
    stored: Option<&BTreeMap<String, JsonData>>,
    field: &'static str,
    limit: RateLimit,
    aggregate: Option<i64>,
) -> ModelCeilings {
    model_ceilings(stored, field, limit, Entity::Team, aggregate).unwrap()
}

fn allocation(limit: RateLimit, allocated: i128, requested: i64, ceiling: i64) -> Rejection {
    Rejection::Allocation {
        limit,
        entity: Entity::Team,
        allocated,
        requested,
        ceiling,
    }
}

#[test]
fn model_top_level_precedence() {
    let stored = keys(r#"[{"token": "k1", "metadata": {"model_rpm_limit": {"gpt-4": 8}}}]"#);
    let request = generate(
        r#"{"model_rpm_limit": {"gpt-4": 4}, "metadata": {"model_rpm_limit": {"gpt-4": 50}}}"#,
    );
    let rpm = ceilings(None, MODEL_RPM_FIELD, RateLimit::Rpm, Some(10));
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    let result = check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm);
    assert_eq!(rejected(result), allocation(RateLimit::Rpm, 8, 4, 10));
}

#[test]
fn empty_top_level_falls_back() {
    let stored = keys(r#"[{"token": "k1", "metadata": {"model_rpm_limit": {"gpt-4": 8}}}]"#);
    let request =
        generate(r#"{"model_rpm_limit": {}, "metadata": {"model_rpm_limit": {"gpt-4": 4}}}"#);
    let rpm = ceilings(None, MODEL_RPM_FIELD, RateLimit::Rpm, Some(10));
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    let result = check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm);
    assert_eq!(rejected(result), allocation(RateLimit::Rpm, 8, 4, 10));
}

#[test]
fn model_totals() {
    let stored = keys(
        r#"[{"token": "k1", "metadata": {"model_rpm_limit": {"gpt-4": 3, "gpt-3.5": 1}}}, {"token": "k2", "metadata": {"model_rpm_limit": {"gpt-4": 2}}}]"#,
    );
    let request = generate(r#"{"model_rpm_limit": {"gpt-4": 6}}"#);
    let rpm = ceilings(None, MODEL_RPM_FIELD, RateLimit::Rpm, Some(10));
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    let result = check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm);
    assert_eq!(rejected(result), allocation(RateLimit::Rpm, 5, 6, 10));
}

#[test]
fn model_absent_entry_starts_at_zero() {
    let stored = keys(r#"[{"token": "k1", "metadata": {"model_rpm_limit": {"gpt-4": 9}}}]"#);
    let request = generate(r#"{"model_rpm_limit": {"claude-4": 10}}"#);
    let rpm = ceilings(None, MODEL_RPM_FIELD, RateLimit::Rpm, Some(10));
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    let result = check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm);
    result.unwrap();
}

#[test]
fn global_model_ceiling() {
    let stored = keys(r#"[{"token": "k1", "metadata": {"model_rpm_limit": {"gpt-4": 8}}}]"#);
    let request = generate(r#"{"model_rpm_limit": {"gpt-4": 4}}"#);
    let metadata = object(r#"{"model_rpm_limit": {"gpt-4": 3}}"#);
    let rpm = ceilings(Some(&metadata), MODEL_RPM_FIELD, RateLimit::Rpm, Some(10));
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    let result = check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm);
    assert_eq!(rejected(result), allocation(RateLimit::Rpm, 8, 4, 10));
    let bare_keys = keys(r#"[{"token": "k1"}]"#);
    let per_model_rpm = ceilings(Some(&metadata), MODEL_RPM_FIELD, RateLimit::Rpm, None);
    let per_model_result = check_model_limits(
        &bare_keys,
        &KeyLimitRequest::from(&request),
        &per_model_rpm,
        &tpm,
    );
    assert_eq!(
        rejected(per_model_result),
        allocation(RateLimit::Rpm, 0, 4, 3)
    );
}

#[test]
fn model_ceiling_zero_ignored() {
    let metadata = object(r#"{"model_rpm_limit": {"gpt-4": 0}}"#);
    let request = generate(r#"{"model_rpm_limit": {"gpt-4": 5}}"#);
    let stored = keys(r#"[{"token": "k1"}]"#);
    let rpm = ceilings(Some(&metadata), MODEL_RPM_FIELD, RateLimit::Rpm, Some(10));
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm).unwrap();
    let rpm_without_aggregate = ceilings(Some(&metadata), MODEL_RPM_FIELD, RateLimit::Rpm, None);
    check_model_limits(
        &stored,
        &KeyLimitRequest::from(&request),
        &rpm_without_aggregate,
        &tpm,
    )
    .unwrap();
}

#[test]
fn model_ceiling_negative() {
    let metadata = object(r#"{"model_rpm_limit": {"gpt-4": -1}}"#);
    let request = generate(r#"{"model_rpm_limit": {"gpt-4": 0}}"#);
    let stored = keys(r#"[{"token": "k1"}]"#);
    let rpm = ceilings(Some(&metadata), MODEL_RPM_FIELD, RateLimit::Rpm, None);
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    let result = check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm);
    assert_eq!(rejected(result), allocation(RateLimit::Rpm, 0, 0, -1));
}

#[test]
fn model_null_ceiling_ignored() {
    let metadata = object(r#"{"model_rpm_limit": {"gpt-4": null}}"#);
    let request = generate(r#"{"model_rpm_limit": {"gpt-4": 5}}"#);
    let stored = keys(r#"[{"token": "k1"}]"#);
    let rpm = ceilings(Some(&metadata), MODEL_RPM_FIELD, RateLimit::Rpm, None);
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm).unwrap();
}

#[test]
fn model_rpm_before_tpm() {
    let stored = keys(r#"[{"token": "k1"}]"#);
    let request =
        generate(r#"{"model_rpm_limit": {"gpt-4": 11}, "model_tpm_limit": {"gpt-4": 11}}"#);
    let rpm = ceilings(None, MODEL_RPM_FIELD, RateLimit::Rpm, Some(10));
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, Some(10));
    let result = check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm);
    assert_eq!(rejected(result), allocation(RateLimit::Rpm, 0, 11, 10));
    let tpm_request = generate(r#"{"model_tpm_limit": {"gpt-4": 11}}"#);
    let tpm_result = check_model_limits(&stored, &KeyLimitRequest::from(&tpm_request), &rpm, &tpm);
    assert_eq!(rejected(tpm_result), allocation(RateLimit::Tpm, 0, 11, 10));
}

#[test]
fn model_limits_skipped_without_request() {
    let stored = keys(r#"[{"token": "k1", "metadata": {"model_rpm_limit": "broken"}}]"#);
    let request = generate(r#"{"rpm_limit": 5}"#);
    let rpm = ceilings(None, MODEL_RPM_FIELD, RateLimit::Rpm, Some(10));
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm).unwrap();
}

#[test]
fn model_limit_decode_error() {
    let stored = keys(r#"[{"token": "k1"}]"#);
    let request = generate(r#"{"model_rpm_limit": {"gpt-4": "lots"}}"#);
    let rpm = ceilings(None, MODEL_RPM_FIELD, RateLimit::Rpm, Some(10));
    let tpm = ceilings(None, MODEL_TPM_FIELD, RateLimit::Tpm, None);
    let result = check_model_limits(&stored, &KeyLimitRequest::from(&request), &rpm, &tpm);
    match result {
        Err(KeyLimitError::ModelLimits { field, .. }) => assert_eq!(field, "model_rpm_limit"),
        other => panic!("expected a model limit decode error, got {other:?}"),
    }
}

#[test]
fn scalar_totals() {
    let stored = keys(
        r#"[{"token": "k1", "tpm_limit": 3}, {"token": "k2"}, {"token": "k3", "tpm_limit": 0}, {"token": "k4", "tpm_limit": -2}]"#,
    );
    let request = generate(r#"{"tpm_limit": 9}"#);
    check_rate_limits(
        &stored,
        &KeyLimitRequest::from(&request),
        None,
        Some(10),
        Entity::Team,
    )
    .unwrap();
    let over_request = generate(r#"{"tpm_limit": 10}"#);
    let rejection = check_rate_limits(
        &stored,
        &KeyLimitRequest::from(&over_request),
        None,
        Some(10),
        Entity::Team,
    )
    .unwrap_err();
    assert_eq!(rejection, allocation(RateLimit::Tpm, 1, 10, 10));
    assert_eq!(
        text_detail(&rejection),
        "Allocated TPM limit=1 + Key TPM limit=10 is greater than team TPM limit=10"
    );
}

#[test]
fn scalar_tpm_before_rpm() {
    let stored = keys(r#"[{"token": "k1", "tpm_limit": 1, "rpm_limit": 1}]"#);
    let request = generate(r#"{"tpm_limit": 10, "rpm_limit": 10}"#);
    let rejection = check_rate_limits(
        &stored,
        &KeyLimitRequest::from(&request),
        Some(10),
        Some(10),
        Entity::Team,
    )
    .unwrap_err();
    assert_eq!(rejection, allocation(RateLimit::Tpm, 1, 10, 10));
    let rpm_request = generate(r#"{"rpm_limit": 10}"#);
    let rpm_rejection = check_rate_limits(
        &stored,
        &KeyLimitRequest::from(&rpm_request),
        Some(10),
        Some(10),
        Entity::Team,
    )
    .unwrap_err();
    assert_eq!(rpm_rejection, allocation(RateLimit::Rpm, 1, 10, 10));
}

#[test]
fn scalar_zero_ceiling_enforced() {
    let stored = keys(r#"[{"token": "k1", "tpm_limit": 0}]"#);
    let request = generate(r#"{"tpm_limit": 0}"#);
    check_rate_limits(
        &stored,
        &KeyLimitRequest::from(&request),
        None,
        Some(0),
        Entity::Team,
    )
    .unwrap();
    let over_stored = keys(r#"[{"token": "k1", "tpm_limit": 1}]"#);
    let rejection = check_rate_limits(
        &over_stored,
        &KeyLimitRequest::from(&request),
        None,
        Some(0),
        Entity::Team,
    )
    .unwrap_err();
    assert_eq!(rejection, allocation(RateLimit::Tpm, 1, 0, 0));
}

#[test]
fn scalar_missing_ceiling_or_value() {
    let stored = keys(r#"[{"token": "k1", "tpm_limit": 5}]"#);
    let request = generate(r#"{"tpm_limit": 100}"#);
    check_rate_limits(
        &stored,
        &KeyLimitRequest::from(&request),
        None,
        None,
        Entity::Team,
    )
    .unwrap();
    let alias_request = generate(r#"{"key_alias": "alias-2"}"#);
    check_rate_limits(
        &stored,
        &KeyLimitRequest::from(&alias_request),
        Some(10),
        Some(10),
        Entity::Team,
    )
    .unwrap();
}
