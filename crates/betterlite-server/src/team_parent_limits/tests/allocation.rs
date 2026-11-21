use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rstest::rstest;

use super::support::{allocation_text, new_team, organization, rejected, team, update_team};
use crate::team_parent_limits::allocation::{check_model_allocations, check_rate_allocations};
use crate::team_parent_limits::error::TeamLimitError;
use crate::team_parent_limits::request::TeamLimitRequest;

fn org(budget: &str, metadata: &str) -> OrganizationTable {
    organization(&format!(
        r#"{{"organization_id": "org-1", "budget_id": "budget-1", "created_by": "admin", "updated_by": "admin", "litellm_budget_table": {budget}, "metadata": {metadata}}}"#
    ))
}

// gpt holds 6 RPM and 40 TPM across the siblings; the team totals are 40 TPM and 5 RPM
fn siblings() -> Vec<LiteLLMTeamTable> {
    vec![
        team(
            r#"{"team_id": "team-a", "tpm_limit": 40, "rpm_limit": 4, "metadata": {"model_rpm_limit": {"gpt": 4}, "model_tpm_limit": {"gpt": 40}}}"#,
        ),
        team(
            r#"{"team_id": "team-b", "tpm_limit": null, "rpm_limit": 1, "metadata": {"model_rpm_limit": {"gpt": 2, "claude": 9}, "model_tpm_limit": null}}"#,
        ),
        team(r#"{"team_id": "team-c"}"#),
    ]
}

#[derive(Clone, Copy)]
enum Check {
    Model,
    Rate,
}

fn check(
    kind: Check,
    teams: &[LiteLLMTeamTable],
    organization: &OrganizationTable,
    request: &str,
) -> Result<(), TeamLimitError> {
    let parsed = new_team(request);
    let limits = TeamLimitRequest::New(&parsed);
    match kind {
        Check::Model => check_model_allocations(teams, organization, &limits),
        Check::Rate => {
            check_rate_allocations(teams, organization, &limits).map_err(TeamLimitError::from)
        }
    }
}

fn outcome(result: Result<(), TeamLimitError>) -> Option<String> {
    match result {
        Ok(()) => None,
        Err(error) => Some(allocation_text(&rejected(Err(error)))),
    }
}

#[rstest]
#[case::at_ceiling(r#"{"model_rpm_limit": {"gpt": 4}}"#, None)]
#[case::one_over(
    r#"{"model_rpm_limit": {"gpt": 5}}"#,
    Some("Allocated RPM limit=6 + Team RPM limit=5 is greater than organization RPM limit=10")
)]
#[case::unallocated_model(
    r#"{"model_rpm_limit": {"llama": 11}}"#,
    Some("Allocated RPM limit=0 + Team RPM limit=11 is greater than organization RPM limit=10")
)]
fn model_rpm_aggregate(#[case] request: &str, #[case] expected: Option<&str>) {
    let organization = org(r#"{"rpm_limit": 10}"#, "{}");
    let result = check(Check::Model, &siblings(), &organization, request);
    assert_eq!(outcome(result).as_deref(), expected);
}

#[rstest]
#[case::at_ceiling(r#"{"rpm_limit": 100}"#, 3, None)]
#[case::over_without_budget_table(
    "null",
    4,
    Some("Allocated RPM limit=6 + Team RPM limit=4 is greater than organization RPM limit=9")
)]
#[case::aggregate_passes_first(
    r#"{"rpm_limit": 100}"#,
    4,
    Some("Allocated RPM limit=6 + Team RPM limit=4 is greater than organization RPM limit=9")
)]
fn model_rpm_specific(
    #[case] budget: &str,
    #[case] requested: i64,
    #[case] expected: Option<&str>,
) {
    let organization = org(budget, r#"{"model_rpm_limit": {"gpt": 9}}"#);
    let request = format!(r#"{{"model_rpm_limit": {{"gpt": {requested}}}}}"#);
    let result = check(Check::Model, &siblings(), &organization, &request);
    assert_eq!(outcome(result).as_deref(), expected);
}

#[rstest]
#[case::at_ceiling(r#"{"model_tpm_limit": {"gpt": 10}}"#, None)]
#[case::one_over(
    r#"{"model_tpm_limit": {"gpt": 11}}"#,
    Some("Allocated TPM limit=40 + Team TPM limit=11 is greater than organization TPM limit=50")
)]
#[case::rpm_checked_first(
    r#"{"model_rpm_limit": {"gpt": 5}, "model_tpm_limit": {"gpt": 11}}"#,
    Some("Allocated RPM limit=6 + Team RPM limit=5 is greater than organization RPM limit=10")
)]
fn model_tpm_aggregate(#[case] request: &str, #[case] expected: Option<&str>) {
    let organization = org(r#"{"rpm_limit": 10, "tpm_limit": 50}"#, "{}");
    let result = check(Check::Model, &siblings(), &organization, request);
    assert_eq!(outcome(result).as_deref(), expected);
}

#[rstest]
#[case::at_ceiling(5, None)]
#[case::one_over(
    6,
    Some("Allocated TPM limit=40 + Team TPM limit=6 is greater than organization TPM limit=45")
)]
fn model_tpm_specific(#[case] requested: i64, #[case] expected: Option<&str>) {
    let metadata = r#"{"model_tpm_limit": {"gpt": 45}}"#;
    let organization = org(r#"{"tpm_limit": 1000}"#, metadata);
    let request = format!(r#"{{"model_tpm_limit": {{"gpt": {requested}}}}}"#);
    let result = check(Check::Model, &siblings(), &organization, &request);
    assert_eq!(outcome(result).as_deref(), expected);
}

#[rstest]
#[case::zero(r#"{"model_rpm_limit": {"gpt": 0}, "model_tpm_limit": {"gpt": 0}}"#)]
#[case::null(r#"{"model_rpm_limit": {"gpt": null}, "model_tpm_limit": {"gpt": null}}"#)]
#[case::null_map(r#"{"model_rpm_limit": null, "model_tpm_limit": null}"#)]
fn zero_specific_ceiling_skipped(#[case] metadata: &str) {
    let organization = org("null", metadata);
    let request = r#"{"model_rpm_limit": {"gpt": 1000}, "model_tpm_limit": {"gpt": 1000}}"#;
    check(Check::Model, &siblings(), &organization, request).unwrap();
}

#[test]
fn zero_aggregate_ceiling_enforced() {
    let organization = org(r#"{"rpm_limit": 0, "tpm_limit": 0}"#, "{}");
    let model = check(
        Check::Model,
        &[],
        &organization,
        r#"{"model_rpm_limit": {"gpt": 1}}"#,
    );
    assert_eq!(
        outcome(model).as_deref(),
        Some("Allocated RPM limit=0 + Team RPM limit=1 is greater than organization RPM limit=0")
    );
    let total = check(Check::Rate, &[], &organization, r#"{"tpm_limit": 1}"#);
    assert_eq!(
        outcome(total).as_deref(),
        Some("Allocated TPM limit=0 + Team TPM limit=1 is greater than organization TPM limit=0")
    );
    check(
        Check::Rate,
        &[],
        &organization,
        r#"{"tpm_limit": 0, "rpm_limit": 0}"#,
    )
    .unwrap();
}

#[rstest]
#[case::no_budget_table("null", "null")]
#[case::null_budget_limits(r#"{"rpm_limit": null, "tpm_limit": null}"#, "{}")]
#[case::other_model_only("null", r#"{"model_rpm_limit": {"claude": 1}, "model_tpm_limit": {}}"#)]
fn missing_org_budget(#[case] budget: &str, #[case] metadata: &str) {
    let organization = org(budget, metadata);
    let request = r#"{"tpm_limit": 1000000, "rpm_limit": 1000000, "model_rpm_limit": {"gpt": 1000000}, "model_tpm_limit": {"gpt": 1000000}}"#;
    check(Check::Model, &siblings(), &organization, request).unwrap();
    check(Check::Rate, &siblings(), &organization, request).unwrap();
}

#[rstest]
#[case::both_over(
    r#"{"tpm_limit": 11, "rpm_limit": 1}"#,
    Some("Allocated TPM limit=40 + Team TPM limit=11 is greater than organization TPM limit=50")
)]
#[case::rpm_over(
    r#"{"tpm_limit": 10, "rpm_limit": 1}"#,
    Some("Allocated RPM limit=5 + Team RPM limit=1 is greater than organization RPM limit=5")
)]
#[case::both_at_ceiling(r#"{"tpm_limit": 10, "rpm_limit": 0}"#, None)]
#[case::no_request_limits("{}", None)]
fn aggregate_tpm_before_rpm(#[case] request: &str, #[case] expected: Option<&str>) {
    let organization = org(r#"{"rpm_limit": 5, "tpm_limit": 50}"#, "{}");
    let result = check(Check::Rate, &siblings(), &organization, request);
    assert_eq!(outcome(result).as_deref(), expected);
}

#[test]
fn request_model_limits_precede_metadata() {
    let organization = org(r#"{"rpm_limit": 10}"#, "{}");
    let field_wins =
        r#"{"model_rpm_limit": {"gpt": 1}, "metadata": {"model_rpm_limit": {"gpt": 100}}}"#;
    check(Check::Model, &siblings(), &organization, field_wins).unwrap();
    let update = update_team(
        r#"{"team_id": "team-a", "model_rpm_limit": {"gpt": 1}, "metadata": {"model_rpm_limit": {"gpt": 100}}}"#,
    );
    let teams = siblings();
    check_model_allocations(&teams, &organization, &TeamLimitRequest::Update(&update)).unwrap();
    let metadata_only = r#"{"metadata": {"model_rpm_limit": {"gpt": 100}}}"#;
    assert_eq!(
        outcome(check(Check::Model, &teams, &organization, metadata_only)).as_deref(),
        Some(
            "Allocated RPM limit=6 + Team RPM limit=100 is greater than organization RPM limit=10"
        )
    );
}

#[rstest]
#[case::empty_field_reads_metadata(
    r#"{"model_rpm_limit": {}, "metadata": {"model_rpm_limit": {"gpt": 5}}}"#,
    Some("Allocated RPM limit=6 + Team RPM limit=5 is greater than organization RPM limit=10")
)]
#[case::empty_field_without_metadata(r#"{"model_rpm_limit": {}}"#, None)]
#[case::null_metadata_limit(r#"{"metadata": {"model_rpm_limit": null}}"#, None)]
fn empty_model_limits_fall_back(#[case] request: &str, #[case] expected: Option<&str>) {
    let organization = org(r#"{"rpm_limit": 10}"#, "{}");
    let result = check(Check::Model, &siblings(), &organization, request);
    assert_eq!(outcome(result).as_deref(), expected);
}

#[rstest]
#[case::sibling_metadata(
    r#"{"team_id": "bad", "metadata": {"model_rpm_limit": ["gpt"]}}"#,
    "{}"
)]
#[case::organization_metadata(r#"{"team_id": "fine"}"#, r#"{"model_rpm_limit": "gpt"}"#)]
fn malformed_model_limits_named(#[case] sibling: &str, #[case] metadata: &str) {
    let organization = org("null", metadata);
    let result = check(
        Check::Model,
        &[team(sibling)],
        &organization,
        r#"{"model_rpm_limit": {"gpt": 1}}"#,
    );
    assert!(matches!(
        result,
        Err(TeamLimitError::ModelLimits {
            field: "model_rpm_limit",
            ..
        })
    ));
    let skipped = check(Check::Model, &[team(sibling)], &organization, "{}");
    skipped.unwrap();
}
