#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;

use crate::limiter_records::{
    ModelGroupResolver, PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY,
    RateLimitDescriptor, RateLimitDescriptorRateLimit, RateLimitedModel,
};

use super::key_owns_tpm::key_owns_tpm_from_request_metadata;
use super::{ModelDescriptorBuilder, ModelDescriptorsError};

struct Aliases(BTreeMap<String, String>);

impl ModelGroupResolver for Aliases {
    fn resolve(&self, model: &str) -> Option<String> {
        self.0.get(model).cloned()
    }
}

fn builder(window_size: i64, aliases: &[(&str, &str)]) -> ModelDescriptorBuilder {
    let groups = aliases
        .iter()
        .map(|(requested, group)| ((*requested).to_owned(), (*group).to_owned()))
        .collect();
    ModelDescriptorBuilder {
        window_size,
        resolver: Box::new(Aliases(groups)),
    }
}

fn auth(fields: &str) -> UserAPIKeyAuth {
    let values: BTreeMap<String, JsonData> = serde_json::from_str(fields).expect("auth fields");
    UserAPIKeyAuth::construct(AuthInput::Dict(values)).expect("the auth fixture builds")
}

fn metadata(fields: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(fields).expect("request metadata fields")
}

const fn rate_limit(
    requests_per_unit: Option<i64>,
    tokens_per_unit: Option<i64>,
    window_size: i64,
) -> RateLimitDescriptorRateLimit {
    RateLimitDescriptorRateLimit {
        requests_per_unit,
        tokens_per_unit,
        max_parallel_requests: None,
        window_size: Some(window_size),
    }
}

fn model(requested: &str, group: &str) -> RateLimitedModel {
    RateLimitedModel {
        requested: requested.to_owned(),
        group: group.to_owned(),
    }
}

#[test]
fn rated_model_uses_the_resolver_group() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    assert_eq!(
        builder.rated_model(Some("gpt4")),
        Some(model("gpt4", "gpt-4"))
    );
}

#[test]
fn rated_model_falls_back_to_the_requested_name() {
    let builder = builder(60, &[("blank", "")]);
    assert_eq!(
        builder.rated_model(Some("other")),
        Some(model("other", "other"))
    );
    assert_eq!(
        builder.rated_model(Some("blank")),
        Some(model("blank", "blank"))
    );
}

#[test]
fn rated_model_without_a_requested_name_is_none() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    assert_eq!(builder.rated_model(None), None);
    assert_eq!(builder.rated_model(Some("")), None);
}

#[test]
fn organization_descriptor_carries_both_limits_and_the_window() {
    let builder = builder(30, &[]);
    let auth =
        auth(r#"{"org_id": "org-1", "organization_rpm_limit": 10, "organization_tpm_limit": 20}"#);
    assert_eq!(
        builder.organization_descriptors(&auth, None),
        vec![RateLimitDescriptor {
            key: "organization".to_owned(),
            value: "org-1".to_owned(),
            rate_limit: Some(rate_limit(Some(10), Some(20), 30)),
        }]
    );
}

#[test]
fn organization_descriptor_needs_an_id_and_a_limit() {
    let builder = builder(60, &[]);
    let without_limit = auth(r#"{"org_id": "org-1"}"#);
    assert!(
        builder
            .organization_descriptors(&without_limit, None)
            .is_empty()
    );
    let without_id = auth(r#"{"organization_rpm_limit": 10}"#);
    assert!(
        builder
            .organization_descriptors(&without_id, None)
            .is_empty()
    );
}

#[test]
fn organization_descriptor_takes_either_limit_alone() {
    let builder = builder(60, &[]);
    let tpm_only = auth(r#"{"org_id": "org-1", "organization_tpm_limit": 20}"#);
    assert_eq!(
        builder.organization_descriptors(&tpm_only, None),
        vec![RateLimitDescriptor {
            key: "organization".to_owned(),
            value: "org-1".to_owned(),
            rate_limit: Some(rate_limit(None, Some(20), 60)),
        }]
    );
}

#[test]
fn organization_descriptors_add_the_global_one_before_the_model_one() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(
        r#"{"org_id": "org-1", "organization_rpm_limit": 1,
            "organization_metadata": {"model_tpm_limit": {"gpt-4": 2}}}"#,
    );
    let descriptors = builder.organization_descriptors(&auth, Some("gpt4"));
    assert_eq!(descriptors.len(), 2);
    assert_eq!(descriptors[0].key, "organization");
    assert_eq!(descriptors[1].key, "model_per_organization");
}

#[test]
fn model_per_organization_is_built_without_a_global_limit() {
    let builder = builder(45, &[("gpt4", "gpt-4")]);
    let auth =
        auth(r#"{"org_id": "org-1", "organization_metadata": {"model_tpm_limit": {"gpt-4": 50}}}"#);
    assert_eq!(
        builder.organization_descriptors(&auth, Some("gpt4")),
        vec![RateLimitDescriptor {
            key: "model_per_organization".to_owned(),
            value: "org-1:gpt-4".to_owned(),
            rate_limit: Some(rate_limit(None, Some(50), 45)),
        }]
    );
}

#[test]
fn model_per_organization_prefers_the_requested_name() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(
        r#"{"org_id": "org-1", "organization_metadata": {"model_rpm_limit": {"gpt4": 0, "gpt-4": 100}}}"#,
    );
    let descriptors = builder.organization_descriptors(&auth, Some("gpt4"));
    assert_eq!(descriptors.len(), 1);
    assert_eq!(
        descriptors[0]
            .rate_limit
            .as_ref()
            .and_then(|limit| limit.requests_per_unit),
        Some(0)
    );
}

#[test]
fn model_per_organization_prints_an_absent_org_as_none_text() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(r#"{"organization_metadata": {"model_rpm_limit": {"gpt-4": 3}}}"#);
    let descriptors = builder.organization_descriptors(&auth, Some("gpt4"));
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].value, "None:gpt-4");
}

#[test]
fn model_per_key_reads_the_group_limits() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth =
        auth(r#"{"api_key": "sk-oracle", "team_metadata": {"model_rpm_limit": {"gpt-4": 7}}}"#);
    let hashed = account_limits(&auth).hashed_api_key.expect("the key hash");
    let mut descriptors = Vec::new();
    builder
        .key_model_descriptor(&auth, Some("gpt4"), &mut descriptors)
        .expect("the key limits");
    assert_eq!(
        descriptors,
        vec![RateLimitDescriptor {
            key: "model_per_key".to_owned(),
            value: format!("{hashed}:gpt-4"),
            rate_limit: Some(rate_limit(Some(7), None, 60)),
        }]
    );
}

#[test]
fn model_per_key_adds_nothing_without_limits() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth =
        auth(r#"{"api_key": "sk-oracle", "team_metadata": {"model_rpm_limit": {"other": 7}}}"#);
    let mut descriptors = Vec::new();
    builder
        .key_model_descriptor(&auth, Some("gpt4"), &mut descriptors)
        .expect("the key limits");
    assert!(descriptors.is_empty());
}

#[test]
fn model_per_key_adds_nothing_without_a_model() {
    let builder = builder(60, &[]);
    let auth = auth(r#"{"api_key": "sk-oracle"}"#);
    let mut descriptors = Vec::new();
    builder
        .key_model_descriptor(&auth, None, &mut descriptors)
        .expect("the key limits");
    assert!(descriptors.is_empty());
}

#[test]
fn model_per_team_drops_only_the_owned_kind() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(
        r#"{"team_id": "team-1", "metadata": {"model_tpm_limit": {"gpt-4": 0}},
            "team_metadata": {"model_rpm_limit": {"gpt-4": 9}, "model_tpm_limit": {"gpt-4": 11}}}"#,
    );
    let mut descriptors = Vec::new();
    builder.team_model_descriptor(&auth, Some("gpt4"), &mut descriptors);
    assert_eq!(
        descriptors,
        vec![RateLimitDescriptor {
            key: "model_per_team".to_owned(),
            value: "team-1:gpt-4".to_owned(),
            rate_limit: Some(rate_limit(Some(9), None, 60)),
        }]
    );
}

#[test]
fn model_per_team_keeps_both_kinds_without_a_key_limit() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(
        r#"{"team_id": "team-1",
            "team_metadata": {"model_rpm_limit": {"gpt-4": 9}, "model_tpm_limit": {"gpt-4": 11}}}"#,
    );
    let mut descriptors = Vec::new();
    builder.team_model_descriptor(&auth, Some("gpt4"), &mut descriptors);
    assert_eq!(
        descriptors,
        vec![RateLimitDescriptor {
            key: "model_per_team".to_owned(),
            value: "team-1:gpt-4".to_owned(),
            rate_limit: Some(rate_limit(Some(9), Some(11), 60)),
        }]
    );
}

#[test]
fn model_per_team_adds_nothing_without_team_limits() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(r#"{"team_id": "team-1"}"#);
    let mut descriptors = Vec::new();
    builder.team_model_descriptor(&auth, Some("gpt4"), &mut descriptors);
    assert!(descriptors.is_empty());
}

#[test]
fn model_per_team_without_a_team_id_uses_none_text() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(r#"{"team_metadata": {"model_rpm_limit": {"gpt-4": 9}}}"#);
    let mut descriptors = Vec::new();
    builder.team_model_descriptor(&auth, Some("gpt4"), &mut descriptors);
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].value, "None:gpt-4");
}

#[test]
fn model_per_project_reads_project_metadata() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(
        r#"{"project_id": "proj-1", "project_metadata": {"model_tpm_limit": {"gpt4": 4, "gpt-4": 8}}}"#,
    );
    let mut descriptors = Vec::new();
    builder.project_model_descriptor(&auth, Some("gpt4"), &mut descriptors);
    assert_eq!(
        descriptors,
        vec![RateLimitDescriptor {
            key: "model_per_project".to_owned(),
            value: "proj-1:gpt-4".to_owned(),
            rate_limit: Some(rate_limit(None, Some(4), 60)),
        }]
    );
}

#[test]
fn model_per_project_without_a_project_id_uses_none_text() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(r#"{"project_metadata": {"model_rpm_limit": {"gpt-4": 4}}}"#);
    let mut descriptors = Vec::new();
    builder.project_model_descriptor(&auth, Some("gpt4"), &mut descriptors);
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].value, "None:gpt-4");
}

#[test]
fn project_io_descriptors_are_added_in_order() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(
        r#"{"project_id": "proj-1", "project_metadata": {"model_otpm_limit": {"gpt-4": 5}, "model_itpm_limit": {"gpt-4": 6}}}"#,
    );
    let mut descriptors = Vec::new();
    builder.project_io_descriptors(&auth, Some("gpt4"), &mut descriptors);
    assert_eq!(
        descriptors,
        vec![
            RateLimitDescriptor {
                key: PROJECT_ITPM_DESCRIPTOR_KEY.to_owned(),
                value: "proj-1:gpt-4".to_owned(),
                rate_limit: Some(rate_limit(None, Some(6), 60)),
            },
            RateLimitDescriptor {
                key: PROJECT_OTPM_DESCRIPTOR_KEY.to_owned(),
                value: "proj-1:gpt-4".to_owned(),
                rate_limit: Some(rate_limit(None, Some(5), 60)),
            },
        ]
    );
}

#[test]
fn project_io_descriptors_add_only_the_set_kind() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth =
        auth(r#"{"project_id": "proj-1", "project_metadata": {"model_otpm_limit": {"gpt-4": 5}}}"#);
    let mut descriptors = Vec::new();
    builder.project_io_descriptors(&auth, Some("gpt4"), &mut descriptors);
    assert_eq!(
        descriptors,
        vec![RateLimitDescriptor {
            key: PROJECT_OTPM_DESCRIPTOR_KEY.to_owned(),
            value: "proj-1:gpt-4".to_owned(),
            rate_limit: Some(rate_limit(None, Some(5), 60)),
        }]
    );
}

#[test]
fn project_io_descriptors_need_a_project_id() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(r#"{"project_metadata": {"model_itpm_limit": {"gpt-4": 6}}}"#);
    let mut descriptors = Vec::new();
    builder.project_io_descriptors(&auth, Some("gpt4"), &mut descriptors);
    assert!(descriptors.is_empty());
}

#[test]
fn project_io_descriptors_need_a_model() {
    let builder = builder(60, &[]);
    let auth =
        auth(r#"{"project_id": "proj-1", "project_metadata": {"model_itpm_limit": {"gpt-4": 6}}}"#);
    let mut descriptors = Vec::new();
    builder.project_io_descriptors(&auth, Some(""), &mut descriptors);
    assert!(descriptors.is_empty());
}

#[test]
fn project_io_descriptors_add_nothing_without_limits() {
    let builder = builder(60, &[("gpt4", "gpt-4")]);
    let auth = auth(r#"{"project_id": "proj-1"}"#);
    let mut descriptors = Vec::new();
    builder.project_io_descriptors(&auth, Some("gpt4"), &mut descriptors);
    assert!(descriptors.is_empty());
}

#[test]
fn every_descriptor_carries_the_configured_window_size() {
    let builder = builder(7, &[("gpt4", "gpt-4")]);
    let auth = auth(
        r#"{"api_key": "sk-oracle", "org_id": "org-1", "team_id": "team-1", "project_id": "proj-1",
            "organization_rpm_limit": 1, "organization_metadata": {"model_rpm_limit": {"gpt-4": 2}},
            "team_metadata": {"model_rpm_limit": {"gpt-4": 3}},
            "project_metadata": {"model_rpm_limit": {"gpt-4": 4}, "model_itpm_limit": {"gpt-4": 5},
                "model_otpm_limit": {"gpt-4": 6}}}"#,
    );
    let mut descriptors = builder.organization_descriptors(&auth, Some("gpt4"));
    builder
        .key_model_descriptor(&auth, Some("gpt4"), &mut descriptors)
        .expect("the key limits");
    builder.team_model_descriptor(&auth, Some("gpt4"), &mut descriptors);
    builder.project_model_descriptor(&auth, Some("gpt4"), &mut descriptors);
    builder.project_io_descriptors(&auth, Some("gpt4"), &mut descriptors);
    assert_eq!(descriptors.len(), 7);
    assert!(descriptors.iter().all(|descriptor| {
        descriptor
            .rate_limit
            .as_ref()
            .and_then(|limit| limit.window_size)
            == Some(7)
    }));
}

#[test]
fn key_owns_tpm_reads_the_key_metadata() {
    let request_metadata =
        metadata(r#"{"user_api_key_metadata": {"model_tpm_limit": {"gpt-4": 0}}}"#);
    assert!(
        key_owns_tpm_from_request_metadata(&request_metadata, Some(&model("gpt4", "gpt-4")))
            .expect("the key view builds")
    );
}

#[test]
fn key_owns_tpm_reads_the_model_max_budget() {
    let request_metadata = metadata(
        r#"{"user_api_key_model_max_budget": {"gpt-4": {"tpm_limit": 3, "rpm_limit": 1}}}"#,
    );
    assert!(
        key_owns_tpm_from_request_metadata(&request_metadata, Some(&model("gpt4", "gpt-4")))
            .expect("the key view builds")
    );
}

#[test]
fn key_owns_tpm_is_false_without_a_model() {
    let request_metadata =
        metadata(r#"{"user_api_key_metadata": {"model_tpm_limit": {"gpt-4": 1}}}"#);
    assert!(
        !key_owns_tpm_from_request_metadata(&request_metadata, None).expect("no key view is built")
    );
}

#[rstest::rstest]
#[case::missing("{}")]
#[case::null(r#"{"user_api_key_metadata": null, "user_api_key_model_max_budget": null}"#)]
#[case::empty_objects(r#"{"user_api_key_metadata": {}, "user_api_key_model_max_budget": {}}"#)]
#[case::empty_limit_map(
    r#"{"user_api_key_metadata": {"model_tpm_limit": {}}, "user_api_key_model_max_budget": {}}"#
)]
fn key_owns_tpm_treats_missing_none_and_empty_values_as_empty(#[case] fields: &str) {
    let request_metadata = metadata(fields);
    assert!(
        !key_owns_tpm_from_request_metadata(&request_metadata, Some(&model("gpt4", "gpt-4")))
            .expect("the key view builds")
    );
}

#[test]
fn key_owns_tpm_rejects_a_value_of_the_wrong_shape() {
    let request_metadata = metadata(r#"{"user_api_key_metadata": "nope"}"#);
    let outcome =
        key_owns_tpm_from_request_metadata(&request_metadata, Some(&model("gpt4", "gpt-4")));
    assert!(matches!(
        outcome,
        Err(ModelDescriptorsError::RequestMetadata(_))
    ));
}
