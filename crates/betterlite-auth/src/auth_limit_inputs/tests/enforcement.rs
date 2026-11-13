use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;

use crate::auth_limit_inputs::batch::{
    BATCH_LIMIT_KEY, BatchEnqueuedTokenLimitField, BatchLimitRequest, enforce_batch_admin_only,
};
use crate::auth_limit_inputs::error::{AuthLimitInputsError, ManagementEntity};
use crate::auth_limit_inputs::estimates::{
    ESTIMATE_PER_MODEL_FIELD, ESTIMATED_OUTPUT_TOKENS_FIELD, OutputTokenEstimateField,
    OutputTokenEstimateRequest, enforce_estimate_admin_only,
};
use crate::auth_limit_inputs::tests::support;
use crate::resolved_key::auth::UserAPIKeyAuth;

fn caller(role: &str) -> UserAPIKeyAuth {
    support::auth(&[("user_role", support::text(role))])
}

fn estimate_fields(values: &[OutputTokenEstimateField]) -> BTreeSet<OutputTokenEstimateField> {
    values.iter().copied().collect()
}

fn batch_fields(values: &[BatchEnqueuedTokenLimitField]) -> BTreeSet<BatchEnqueuedTokenLimitField> {
    values.iter().copied().collect()
}

fn stored_estimate(value: i64) -> BTreeMap<String, JsonData> {
    support::fields(&[(ESTIMATED_OUTPUT_TOKENS_FIELD, support::number(value))])
}

#[test]
fn omitted_metadata_preserves() {
    let stored = stored_estimate(2000);
    let set = estimate_fields(&[]);
    let request = OutputTokenEstimateRequest {
        metadata: None,
        estimate: None,
        estimate_per_model: None,
        model_fields_set: &set,
    };
    let outcome = enforce_estimate_admin_only(
        &request,
        Some(&stored),
        &caller("internal_user"),
        ManagementEntity::Key,
    );
    assert!(outcome.is_ok());
}

#[test]
fn explicit_null_metadata_clears() {
    let stored = stored_estimate(2000);
    let set = estimate_fields(&[OutputTokenEstimateField::Metadata]);
    let request = OutputTokenEstimateRequest {
        metadata: None,
        estimate: None,
        estimate_per_model: None,
        model_fields_set: &set,
    };
    let outcome = enforce_estimate_admin_only(
        &request,
        Some(&stored),
        &caller("internal_user"),
        ManagementEntity::Key,
    );
    assert!(matches!(
        outcome.unwrap_err(),
        AuthLimitInputsError::EstimateAdminOnly {
            entity: ManagementEntity::Key
        }
    ));
}

#[test]
fn top_level_nonnull_overlays() {
    let stored = support::fields(&[
        (ESTIMATED_OUTPUT_TOKENS_FIELD, support::number(2000)),
        ("other", support::text("keep")),
    ]);
    let metadata = support::fields(&[("other", support::text("keep"))]);
    let set = estimate_fields(&[OutputTokenEstimateField::Metadata]);
    let unchanged = OutputTokenEstimateRequest {
        metadata: Some(&metadata),
        estimate: Some(2000),
        estimate_per_model: None,
        model_fields_set: &set,
    };
    assert!(
        enforce_estimate_admin_only(
            &unchanged,
            Some(&stored),
            &caller("internal_user"),
            ManagementEntity::Key,
        )
        .is_ok()
    );
    let changed = OutputTokenEstimateRequest {
        metadata: Some(&metadata),
        estimate: Some(1),
        estimate_per_model: None,
        model_fields_set: &set,
    };
    assert!(
        enforce_estimate_admin_only(
            &changed,
            Some(&stored),
            &caller("internal_user"),
            ManagementEntity::Key,
        )
        .is_err()
    );
}

#[test]
fn top_level_null_no_clear() {
    let stored = stored_estimate(2000);
    let set = estimate_fields(&[OutputTokenEstimateField::Estimate]);
    let request = OutputTokenEstimateRequest {
        metadata: None,
        estimate: None,
        estimate_per_model: None,
        model_fields_set: &set,
    };
    let outcome = enforce_estimate_admin_only(
        &request,
        Some(&stored),
        &caller("internal_user"),
        ManagementEntity::Key,
    );
    assert!(outcome.is_ok());
}

#[test]
fn admin_estimate_bypass() {
    let stored = stored_estimate(2000);
    let set = estimate_fields(&[]);
    let request = OutputTokenEstimateRequest {
        metadata: None,
        estimate: Some(1),
        estimate_per_model: None,
        model_fields_set: &set,
    };
    assert!(
        enforce_estimate_admin_only(
            &request,
            Some(&stored),
            &caller("proxy_admin"),
            ManagementEntity::Team,
        )
        .is_ok()
    );
}

#[test]
fn nonadmin_identical_allowed() {
    let stored = stored_estimate(2000);
    let set = estimate_fields(&[OutputTokenEstimateField::Estimate]);
    let request = OutputTokenEstimateRequest {
        metadata: None,
        estimate: Some(2000),
        estimate_per_model: None,
        model_fields_set: &set,
    };
    assert!(
        enforce_estimate_admin_only(
            &request,
            Some(&stored),
            &caller("internal_user"),
            ManagementEntity::Key,
        )
        .is_ok()
    );
}

#[test]
fn estimate_changed_403() {
    let stored = stored_estimate(2000);
    let set = estimate_fields(&[OutputTokenEstimateField::Estimate]);
    let request = OutputTokenEstimateRequest {
        metadata: None,
        estimate: Some(1),
        estimate_per_model: None,
        model_fields_set: &set,
    };
    let failure = enforce_estimate_admin_only(
        &request,
        Some(&stored),
        &caller("internal_user"),
        ManagementEntity::Key,
    );
    let message = failure.unwrap_err().to_string();
    assert!(message.contains(ESTIMATED_OUTPUT_TOKENS_FIELD));
    assert!(message.contains(ESTIMATE_PER_MODEL_FIELD));
    assert!(message.contains("max_tokens"));
    assert!(message.contains("key"));
}

#[test]
fn estimate_removed_403() {
    let stored = support::fields(&[
        (ESTIMATED_OUTPUT_TOKENS_FIELD, support::number(2000)),
        ("other", support::text("keep")),
    ]);
    let metadata = support::fields(&[("other", support::text("keep"))]);
    let set = estimate_fields(&[OutputTokenEstimateField::Metadata]);
    let request = OutputTokenEstimateRequest {
        metadata: Some(&metadata),
        estimate: None,
        estimate_per_model: None,
        model_fields_set: &set,
    };
    let outcome = enforce_estimate_admin_only(
        &request,
        Some(&stored),
        &caller("internal_user"),
        ManagementEntity::Key,
    );
    assert!(matches!(
        outcome.unwrap_err(),
        AuthLimitInputsError::EstimateAdminOnly { .. }
    ));
}

#[test]
fn batch_omitted_preserves() {
    let stored = support::fields(&[(BATCH_LIMIT_KEY, support::number(100000))]);
    let set = batch_fields(&[]);
    let request = BatchLimitRequest {
        metadata: None,
        model_fields_set: &set,
    };
    assert!(
        enforce_batch_admin_only(
            &request,
            Some(&stored),
            &caller("internal_user"),
            ManagementEntity::Key,
        )
        .is_ok()
    );
}

#[test]
fn batch_identical_allowed() {
    let stored = support::fields(&[(BATCH_LIMIT_KEY, support::number(100000))]);
    let metadata = support::fields(&[(BATCH_LIMIT_KEY, support::number(100000))]);
    let set = batch_fields(&[BatchEnqueuedTokenLimitField::Metadata]);
    let request = BatchLimitRequest {
        metadata: Some(&metadata),
        model_fields_set: &set,
    };
    assert!(
        enforce_batch_admin_only(
            &request,
            Some(&stored),
            &caller("internal_user"),
            ManagementEntity::Key,
        )
        .is_ok()
    );
}

#[test]
fn batch_changed_403() {
    let stored = support::fields(&[(BATCH_LIMIT_KEY, support::number(100000))]);
    let metadata = support::fields(&[(BATCH_LIMIT_KEY, support::number(200000))]);
    let set = batch_fields(&[BatchEnqueuedTokenLimitField::Metadata]);
    let request = BatchLimitRequest {
        metadata: Some(&metadata),
        model_fields_set: &set,
    };
    let outcome = enforce_batch_admin_only(
        &request,
        Some(&stored),
        &caller("internal_user"),
        ManagementEntity::Key,
    );
    let message = outcome.unwrap_err().to_string();
    assert!(message.contains(BATCH_LIMIT_KEY));
    assert!(message.contains("replaces the standard rate limit checks"));
}

#[test]
fn batch_removed_403() {
    let stored = support::fields(&[(BATCH_LIMIT_KEY, support::number(100000))]);
    let metadata = support::fields(&[]);
    let set = batch_fields(&[BatchEnqueuedTokenLimitField::Metadata]);
    let request = BatchLimitRequest {
        metadata: Some(&metadata),
        model_fields_set: &set,
    };
    let outcome = enforce_batch_admin_only(
        &request,
        Some(&stored),
        &caller("internal_user"),
        ManagementEntity::Key,
    );
    assert!(matches!(
        outcome.unwrap_err(),
        AuthLimitInputsError::BatchLimitAdminOnly { .. }
    ));
}

#[test]
fn batch_admin_bypass() {
    let stored = support::fields(&[(BATCH_LIMIT_KEY, support::number(100000))]);
    let metadata = support::fields(&[]);
    let set = batch_fields(&[BatchEnqueuedTokenLimitField::Metadata]);
    let request = BatchLimitRequest {
        metadata: Some(&metadata),
        model_fields_set: &set,
    };
    assert!(
        enforce_batch_admin_only(
            &request,
            Some(&stored),
            &caller("proxy_admin"),
            ManagementEntity::Team,
        )
        .is_ok()
    );
}
