use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use crate::auth_limit_inputs::deployment::DeploymentLimitValue;
use crate::auth_limit_inputs::error::AuthLimitInputsError;
use crate::auth_limit_inputs::key_limits::{ModelRateLimitKey, get_key_own_model_rate_limit};
use crate::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, get_key_mcp_rpm_limit, get_key_tag_rpm_limit, get_team_mcp_rpm_limit,
    model_limit_from_metadata,
};
use crate::auth_limit_inputs::tests::support;

fn limit_field(name: &str, entries: &[(&str, i64)]) -> JsonData {
    support::object(&[(name, support::number_object(entries))])
}

#[test]
fn key_metadata_truthy_first() {
    let auth = support::auth(&[
        ("metadata", limit_field("model_rpm_limit", &[("gpt-4", 10)])),
        (
            "model_max_budget",
            support::object(&[(
                "gpt-4",
                support::object(&[("rpm_limit", support::number(5))]),
            )]),
        ),
        (
            "team_metadata",
            limit_field("model_rpm_limit", &[("gpt-4", 7)]),
        ),
    ]);
    let catalog = support::catalog(
        "gpt-4",
        vec![support::deployment(
            Some(DeploymentLimitValue::Integer(1)),
            None,
        )],
    );
    let limits = support::model_limits(&auth, Some("gpt-4"), Some(&catalog)).unwrap();
    assert_eq!(limits, Some(support::limits(&[("gpt-4", 10)])));
}

#[test]
fn empty_key_map_falls_through() {
    let auth = support::auth(&[
        ("metadata", limit_field("model_rpm_limit", &[])),
        (
            "team_metadata",
            limit_field("model_rpm_limit", &[("gpt-4", 7)]),
        ),
    ]);
    let limits = support::model_limits(&auth, Some("gpt-4"), None).unwrap();
    assert_eq!(limits, Some(support::limits(&[("gpt-4", 7)])));
}

#[test]
fn budget_nested_dict_nonnull() {
    let auth = support::auth(&[(
        "model_max_budget",
        support::object(&[
            (
                "gpt-4",
                support::object(&[
                    ("rpm_limit", support::number(5)),
                    ("tpm_limit", JsonData::Null),
                ]),
            ),
            ("gpt-3", support::object(&[("rpm_limit", JsonData::Null)])),
            ("other", support::text("not-a-dict")),
        ]),
    )]);
    let rpm = support::model_limits(&auth, Some("gpt-4"), None).unwrap();
    assert_eq!(rpm, Some(support::limits(&[("gpt-4", 5)])));
    let tpm = support::tpm_limits(&auth, Some("gpt-4"), None).unwrap();
    assert_eq!(tpm, None);
}

#[test]
fn team_empty_map_wins() {
    let auth = support::auth(&[("team_metadata", limit_field("model_rpm_limit", &[]))]);
    let catalog = support::catalog(
        "gpt-4",
        vec![support::deployment(
            Some(DeploymentLimitValue::Integer(3)),
            None,
        )],
    );
    let limits = support::model_limits(&auth, Some("gpt-4"), Some(&catalog)).unwrap();
    assert_eq!(limits, Some(BTreeMap::new()));
}

#[test]
fn deployment_minimum() {
    let auth = support::auth(&[]);
    let catalog = support::catalog(
        "gpt-4",
        vec![
            support::deployment(
                Some(DeploymentLimitValue::Integer(10)),
                Some(DeploymentLimitValue::Integer(9)),
            ),
            support::deployment(
                Some(DeploymentLimitValue::Integer(4)),
                Some(DeploymentLimitValue::Integer(7)),
            ),
            support::deployment(None, None),
        ],
    );
    let rpm = support::model_limits(&auth, Some("gpt-4"), Some(&catalog)).unwrap();
    assert_eq!(rpm, Some(support::limits(&[("gpt-4", 4)])));
    let tpm = support::tpm_limits(&auth, Some("gpt-4"), Some(&catalog)).unwrap();
    assert_eq!(tpm, Some(support::limits(&[("gpt-4", 7)])));
}

#[test]
fn absent_router_or_models() {
    let auth = support::auth(&[]);
    assert_eq!(
        support::model_limits(&auth, Some("gpt-4"), None).unwrap(),
        None
    );
    let absent = support::no_router();
    assert_eq!(
        support::model_limits(&auth, Some("gpt-4"), Some(&absent)).unwrap(),
        None
    );
    let other_model = support::catalog(
        "gpt-3",
        vec![support::deployment(
            Some(DeploymentLimitValue::Integer(5)),
            None,
        )],
    );
    assert_eq!(
        support::model_limits(&auth, Some("gpt-4"), Some(&other_model)).unwrap(),
        None
    );
    let unset = support::catalog("gpt-4", vec![support::deployment(None, None)]);
    assert_eq!(
        support::model_limits(&auth, Some("gpt-4"), Some(&unset)).unwrap(),
        None
    );
    assert_eq!(
        support::model_limits(&auth, None, Some(&other_model)).unwrap(),
        None
    );
}

#[rstest]
#[case::integer_string(DeploymentLimitValue::Text("12".to_owned()), 12)]
#[case::bytes(DeploymentLimitValue::Bytes(b"12".to_vec()), 12)]
#[case::bool_true(DeploymentLimitValue::Bool(true), 1)]
#[case::bool_false(DeploymentLimitValue::Bool(false), 0)]
#[case::integer(DeploymentLimitValue::Integer(12), 12)]
#[case::unsigned(DeploymentLimitValue::Unsigned(12), 12)]
fn integer_string_bytes_bool(value: DeploymentLimitValue, expected: i64) {
    let catalog = support::catalog("gpt-4", vec![support::deployment(Some(value), None)]);
    let limits = support::model_limits(&support::auth(&[]), Some("gpt-4"), Some(&catalog));
    assert_eq!(
        limits.unwrap(),
        Some(support::limits(&[("gpt-4", expected)]))
    );
}

#[rstest]
#[case::truncates_positive(1.9, 1)]
#[case::truncates_negative(-1.9, -1)]
#[case::truncates_half_up(2.5, 2)]
#[case::truncates_half_down(-2.5, -2)]
#[case::keeps_whole(3.0, 3)]
fn decimal_truncation(value: f64, expected: i64) {
    let catalog = support::catalog(
        "gpt-4",
        vec![support::deployment(
            Some(DeploymentLimitValue::Float(value)),
            None,
        )],
    );
    let limits = support::model_limits(&support::auth(&[]), Some("gpt-4"), Some(&catalog));
    assert_eq!(
        limits.unwrap(),
        Some(support::limits(&[("gpt-4", expected)]))
    );
}

#[test]
fn malformed_limit_skipped() {
    let auth = support::auth(&[]);
    let catalog = support::catalog(
        "gpt-4",
        vec![
            support::deployment(Some(DeploymentLimitValue::Text("abc".to_owned())), None),
            support::deployment(Some(DeploymentLimitValue::Bytes(b"\xff".to_vec())), None),
            support::deployment(Some(DeploymentLimitValue::Unsupported), None),
        ],
    );
    assert_eq!(
        support::model_limits(&auth, Some("gpt-4"), Some(&catalog)).unwrap(),
        None
    );
    let mixed = support::catalog(
        "gpt-4",
        vec![
            support::deployment(Some(DeploymentLimitValue::Text("abc".to_owned())), None),
            support::deployment(Some(DeploymentLimitValue::Integer(5)), None),
        ],
    );
    assert_eq!(
        support::model_limits(&auth, Some("gpt-4"), Some(&mixed)).unwrap(),
        Some(support::limits(&[("gpt-4", 5)]))
    );
}

#[rstest]
#[case::infinite(DeploymentLimitValue::Float(f64::INFINITY))]
#[case::unsigned_too_large(DeploymentLimitValue::Unsigned(u64::MAX))]
fn overflow_propagates(value: DeploymentLimitValue) {
    let catalog = support::catalog("gpt-4", vec![support::deployment(Some(value), None)]);
    let failure = support::model_limits(&support::auth(&[]), Some("gpt-4"), Some(&catalog));
    assert!(matches!(
        failure.unwrap_err(),
        AuthLimitInputsError::LimitOverflow { .. }
    ));
}

#[test]
fn zero_negative_kept() {
    let catalog = support::catalog(
        "gpt-4",
        vec![
            support::deployment(Some(DeploymentLimitValue::Integer(0)), None),
            support::deployment(Some(DeploymentLimitValue::Integer(-5)), None),
        ],
    );
    let deployment = support::model_limits(&support::auth(&[]), Some("gpt-4"), Some(&catalog));
    assert_eq!(deployment.unwrap(), Some(support::limits(&[("gpt-4", -5)])));
    let metadata = support::auth(&[("metadata", limit_field("model_rpm_limit", &[("gpt-4", 0)]))]);
    let key = get_key_own_model_rate_limit(&metadata, ModelRateLimitKey::Rpm);
    assert_eq!(key, Some(support::limits(&[("gpt-4", 0)])));
}

#[test]
fn returned_map_not_model_filtered() {
    let auth = support::auth(&[(
        "metadata",
        limit_field("model_rpm_limit", &[("gpt-4", 10), ("gpt-3", 5)]),
    )]);
    let limits = support::model_limits(&auth, Some("gpt-4"), None).unwrap();
    assert_eq!(
        limits,
        Some(support::limits(&[("gpt-4", 10), ("gpt-3", 5)]))
    );
    let budget = support::auth(&[(
        "model_max_budget",
        support::object(&[
            (
                "gpt-4",
                support::object(&[("rpm_limit", support::number(10))]),
            ),
            (
                "gpt-3",
                support::object(&[("rpm_limit", support::number(5))]),
            ),
        ]),
    )]);
    let limits = support::model_limits(&budget, Some("gpt-4"), None).unwrap();
    assert_eq!(
        limits,
        Some(support::limits(&[("gpt-4", 10), ("gpt-3", 5)]))
    );
}

#[test]
fn tenant_accessor_fields() {
    let auth = support::auth(&[
        (
            "team_metadata",
            limit_field("model_rpm_limit", &[("gpt-4", 1)]),
        ),
        (
            "organization_metadata",
            support::object(&[
                ("model_rpm_limit", support::number_object(&[("gpt-4", 2)])),
                ("model_tpm_limit", support::number_object(&[("gpt-4", 3)])),
                ("model_itpm_limit", support::number_object(&[("gpt-4", 4)])),
                ("model_otpm_limit", support::number_object(&[("gpt-4", 5)])),
            ]),
        ),
        (
            "project_metadata",
            limit_field("model_tpm_limit", &[("gpt-4", 6)]),
        ),
    ]);
    let team = model_limit_from_metadata(&auth, MetadataAccessor::Team, ModelRateLimitKey::Rpm);
    assert_eq!(team, Some(support::limits(&[("gpt-4", 1)])));
    let org_rpm = model_limit_from_metadata(
        &auth,
        MetadataAccessor::Organization,
        ModelRateLimitKey::Rpm,
    );
    assert_eq!(org_rpm, Some(support::limits(&[("gpt-4", 2)])));
    let org_itpm = model_limit_from_metadata(
        &auth,
        MetadataAccessor::Organization,
        ModelRateLimitKey::Itpm,
    );
    assert_eq!(org_itpm, Some(support::limits(&[("gpt-4", 4)])));
    let org_otpm = model_limit_from_metadata(
        &auth,
        MetadataAccessor::Organization,
        ModelRateLimitKey::Otpm,
    );
    assert_eq!(org_otpm, Some(support::limits(&[("gpt-4", 5)])));
    let project =
        model_limit_from_metadata(&auth, MetadataAccessor::Project, ModelRateLimitKey::Tpm);
    assert_eq!(project, Some(support::limits(&[("gpt-4", 6)])));
    let absent = support::auth(&[]);
    assert_eq!(
        model_limit_from_metadata(&absent, MetadataAccessor::Team, ModelRateLimitKey::Rpm),
        None
    );
    let empty = support::auth(&[("team_metadata", support::object(&[]))]);
    assert_eq!(
        model_limit_from_metadata(&empty, MetadataAccessor::Team, ModelRateLimitKey::Rpm),
        None
    );
    let missing = support::auth(&[(
        "project_metadata",
        limit_field("model_rpm_limit", &[("gpt-4", 1)]),
    )]);
    assert_eq!(
        model_limit_from_metadata(&missing, MetadataAccessor::Project, ModelRateLimitKey::Tpm),
        None
    );
}

#[test]
fn key_mcp_empty_wins() {
    let auth = support::auth(&[
        ("metadata", limit_field("mcp_rpm_limit", &[])),
        (
            "team_metadata",
            limit_field("mcp_rpm_limit", &[("github", 7)]),
        ),
    ]);
    assert_eq!(get_key_mcp_rpm_limit(&auth), Some(BTreeMap::new()));
}

#[test]
fn team_mcp_fallback() {
    let auth = support::auth(&[(
        "team_metadata",
        limit_field("mcp_rpm_limit", &[("github", 7)]),
    )]);
    assert_eq!(
        get_key_mcp_rpm_limit(&auth),
        Some(support::limits(&[("github", 7)]))
    );
    assert_eq!(
        get_team_mcp_rpm_limit(&auth),
        Some(support::limits(&[("github", 7)]))
    );
    assert_eq!(get_key_mcp_rpm_limit(&support::auth(&[])), None);
    let key_only = support::auth(&[("metadata", limit_field("mcp_rpm_limit", &[("github", 3)]))]);
    assert_eq!(
        get_key_mcp_rpm_limit(&key_only),
        Some(support::limits(&[("github", 3)]))
    );
    assert_eq!(get_team_mcp_rpm_limit(&key_only), None);
}

#[test]
fn tag_key_only() {
    let auth = support::auth(&[
        ("metadata", limit_field("tag_rpm_limit", &[("tag-1", 5)])),
        (
            "team_metadata",
            limit_field("tag_rpm_limit", &[("tag-2", 9)]),
        ),
    ]);
    assert_eq!(
        get_key_tag_rpm_limit(&auth),
        Some(support::limits(&[("tag-1", 5)]))
    );
    let team_only = support::auth(&[(
        "team_metadata",
        limit_field("tag_rpm_limit", &[("tag-2", 9)]),
    )]);
    assert_eq!(get_key_tag_rpm_limit(&team_only), None);
}

#[test]
fn model_tpm_reader_uses_tpm_field() {
    let auth = support::auth(&[("metadata", limit_field("model_tpm_limit", &[("gpt-4", 11)]))]);
    let limits = support::tpm_limits(&auth, Some("gpt-4"), None).unwrap();
    assert_eq!(limits, Some(support::limits(&[("gpt-4", 11)])));
    assert_eq!(
        support::model_limits(&auth, Some("gpt-4"), None).unwrap(),
        None
    );
}

#[test]
fn key_metadata_prefers_the_same_rate_limit_field() {
    let auth = support::auth(&[("metadata", limit_field("model_rpm_limit", &[("gpt-4", 2)]))]);
    let limits = get_key_own_model_rate_limit(&auth, ModelRateLimitKey::Rpm);
    assert_eq!(limits, Some(support::limits(&[("gpt-4", 2)])));
    let other = get_key_own_model_rate_limit(&auth, ModelRateLimitKey::Tpm);
    assert_eq!(other, None);
}
