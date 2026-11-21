use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::organization::OrganizationTable;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    allocation_text, close, count, database, error_text, new_team, organization, rejected,
    update_team,
};
use crate::team_parent_limits::error::{REJECTION_STATUS, Rejection, TeamLimitError};
use crate::team_parent_limits::organization::check_org_team_limits;
use crate::team_parent_limits::request::TeamLimitRequest;

fn org(models: &str, budget: &str) -> OrganizationTable {
    organization(&format!(
        r#"{{"organization_id": "org-1", "budget_id": "budget-1", "created_by": "admin", "updated_by": "admin", "models": {models}, "litellm_budget_table": {budget}}}"#
    ))
}

async fn check_new(organization: &OrganizationTable, request: &str) -> Result<(), TeamLimitError> {
    let parsed = new_team(request);
    check_org_team_limits(organization, &TeamLimitRequest::from(&parsed), None).await
}

async fn check_update(
    organization: &OrganizationTable,
    request: &str,
) -> Result<(), TeamLimitError> {
    let parsed = update_team(request);
    check_org_team_limits(organization, &TeamLimitRequest::from(&parsed), None).await
}

fn name(model: &str) -> JsonData {
    JsonData::String(model.to_owned())
}

#[rstest]
#[case::equal(r#"{"max_budget": 100}"#, r#"{"max_budget": 100}"#, None)]
#[case::above(r#"{"max_budget": 100}"#, r#"{"max_budget": 100.5}"#, Some((1005, 1)))]
#[case::no_ceiling(r#"{"max_budget": null}"#, r#"{"max_budget": 1000000}"#, None)]
#[case::no_budget_table("null", r#"{"max_budget": 1000000}"#, None)]
#[case::no_request_budget(r#"{"max_budget": 1}"#, "{}", None)]
#[tokio::test]
async fn org_budget_boundary(
    #[case] budget: &str,
    #[case] request: &str,
    #[case] rejected_at: Option<(i64, u32)>,
) {
    let organization = org("[]", budget);
    let result = check_new(&organization, request).await;
    let Some((mantissa, scale)) = rejected_at else {
        result.unwrap();
        return;
    };
    let rejection = rejected(result);
    assert_eq!(
        rejection,
        Rejection::OrganizationBudget {
            requested: Decimal::new(mantissa, scale),
            ceiling: Decimal::from(100_i64),
            organization: "org-1".to_owned(),
        }
    );
    assert!(error_text(&rejection).contains("max_budget"));
    assert_eq!(REJECTION_STATUS.as_u16(), 400);
}

#[rstest]
#[case::listed(r#"{"models": ["gpt", "claude"]}"#, None)]
#[case::unlisted(r#"{"models": ["gpt", "llama"]}"#, Some("llama"))]
#[case::no_models("{}", None)]
#[tokio::test]
async fn org_model_allowlist(#[case] request: &str, #[case] unlisted: Option<&str>) {
    let organization = org(r#"["gpt", "claude"]"#, "null");
    let result = check_new(&organization, request).await;
    let Some(model) = unlisted else {
        result.unwrap();
        return;
    };
    let rejection = rejected(result);
    assert_eq!(
        rejection,
        Rejection::OrganizationModel {
            model: name(model),
            allowed: vec!["gpt".to_owned(), "claude".to_owned()],
            organization: "org-1".to_owned(),
        }
    );
    error_text(&rejection);
    let update = r#"{"team_id": "team-a", "models": [5]}"#;
    let numeric = rejected(check_update(&organization, update).await);
    assert!(matches!(
        numeric,
        Rejection::OrganizationModel {
            model: JsonData::Number(_),
            ..
        }
    ));
}

#[tokio::test]
async fn org_all_proxy_models() {
    let organization = org(r#"["gpt", "all-proxy-models"]"#, "null");
    check_new(&organization, r#"{"models": ["llama", "anything"]}"#)
        .await
        .unwrap();
}

#[tokio::test]
async fn org_empty_models() {
    let organization = org("[]", "null");
    check_new(&organization, r#"{"models": ["llama"]}"#)
        .await
        .unwrap();
    check_update(
        &organization,
        r#"{"team_id": "team-a", "models": ["llama", 5]}"#,
    )
    .await
    .unwrap();
}

#[rstest]
#[case::equal(100, false)]
#[case::above(101, true)]
#[tokio::test]
async fn org_direct_tpm(#[case] requested: i64, #[case] rejects: bool) {
    let organization = org("[]", r#"{"tpm_limit": 100, "rpm_limit": null}"#);
    let request = format!(r#"{{"tpm_limit": {requested}, "rpm_limit": 1000000}}"#);
    let result = check_new(&organization, &request).await;
    if !rejects {
        result.unwrap();
        return;
    }
    let rejection = rejected(result);
    let organization_id = "org-1".to_owned();
    let expected = Rejection::OrganizationTpm {
        requested,
        ceiling: 100,
        organization: organization_id,
    };
    assert_eq!(rejection, expected);
    assert!(error_text(&rejection).contains("tpm_limit"));
}

#[rstest]
#[case::equal(10, false)]
#[case::above(11, true)]
#[tokio::test]
async fn org_direct_rpm(#[case] requested: i64, #[case] rejects: bool) {
    let organization = org("[]", r#"{"rpm_limit": 10, "tpm_limit": null}"#);
    let request = format!(r#"{{"rpm_limit": {requested}, "tpm_limit": 1000000}}"#);
    let result = check_new(&organization, &request).await;
    if !rejects {
        result.unwrap();
        return;
    }
    let rejection = rejected(result);
    let organization_id = "org-1".to_owned();
    let expected = Rejection::OrganizationRpm {
        requested,
        ceiling: 10,
        organization: organization_id,
    };
    assert_eq!(rejection, expected);
    assert!(error_text(&rejection).contains("rpm_limit"));
}

#[rstest]
#[case::budget_first(
    r#"{"max_budget": 2, "models": ["llama"], "tpm_limit": 2, "rpm_limit": 2}"#,
    "budget"
)]
#[case::models_second(
    r#"{"max_budget": 1, "models": ["llama"], "tpm_limit": 2, "rpm_limit": 2}"#,
    "models"
)]
#[case::tpm_third(
    r#"{"max_budget": 1, "models": ["gpt"], "tpm_limit": 2, "rpm_limit": 2}"#,
    "tpm"
)]
#[case::rpm_last(
    r#"{"max_budget": 1, "models": ["gpt"], "tpm_limit": 1, "rpm_limit": 2}"#,
    "rpm"
)]
#[tokio::test]
async fn org_failure_order(#[case] request: &str, #[case] first: &str) {
    let budget = r#"{"max_budget": 1, "tpm_limit": 1, "rpm_limit": 1}"#;
    let organization = org(r#"["gpt"]"#, budget);
    let rejection = rejected(check_new(&organization, request).await);
    let kind = match rejection {
        Rejection::OrganizationBudget { .. } => "budget",
        Rejection::OrganizationModel { .. } => "models",
        Rejection::OrganizationTpm { .. } => "tpm",
        Rejection::OrganizationRpm { .. } => "rpm",
        Rejection::Allocation { .. }
        | Rejection::UserBudget { .. }
        | Rejection::UserModel { .. }
        | Rejection::UserTpm { .. }
        | Rejection::UserRpm { .. } => "other",
    };
    assert_eq!(kind, first);
}

#[rstest]
#[case::no_limit_types(r#"{"tpm_limit": 1}"#)]
#[case::field_wins_over_metadata(
    r#"{"rpm_limit_type": "best_effort_throughput", "metadata": {"rpm_limit_type": "guaranteed_throughput"}}"#
)]
#[case::best_effort_metadata(r#"{"metadata": {"tpm_limit_type": "best_effort_throughput"}}"#)]
#[case::non_string_metadata(r#"{"metadata": {"tpm_limit_type": true}}"#)]
#[tokio::test]
async fn no_guarantee_no_query(#[case] request: &str) {
    let organization = org("[]", "null");
    check_new(&organization, request).await.unwrap();
}

#[rstest]
#[case::update_metadata_rpm(
    false,
    r#"{"team_id": "team-a", "metadata": {"rpm_limit_type": "guaranteed_throughput"}}"#
)]
#[case::new_metadata_tpm(true, r#"{"metadata": {"tpm_limit_type": "guaranteed_throughput"}}"#)]
#[case::new_field_tpm(true, r#"{"tpm_limit_type": "guaranteed_throughput"}"#)]
#[tokio::test]
async fn metadata_guarantee_fallback(#[case] create: bool, #[case] request: &str) {
    let organization = org("[]", "null");
    let result = if create {
        check_new(&organization, request).await
    } else {
        check_update(&organization, request).await
    };
    assert!(matches!(
        result,
        Err(TeamLimitError::Database(DbError::NoDbConnected))
    ));
    let mut unnamed = org("[]", "null");
    unnamed.organization_id = None;
    let missing = check_new(&unnamed, r#"{"tpm_limit_type": "guaranteed_throughput"}"#).await;
    assert!(matches!(missing, Err(TeamLimitError::MissingOrganization)));
}

const SIBLINGS: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('budget-1', 'admin', 'admin');
INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by)
VALUES ('org-1', 'one', 'budget-1', 'admin', 'admin'), ('org-2', 'two', 'budget-1', 'admin', 'admin');
INSERT INTO "LiteLLM_TeamTable" (team_id, organization_id, admins, members, models, members_with_roles, tpm_limit, rpm_limit, metadata)
VALUES
    ('team-a', 'org-1', '{}', '{}', '{}', '[]', 50, 2, '{"model_rpm_limit": {"gpt": 3}}'),
    ('team-b', 'org-1', '{}', '{}', '{}', '[]', 30, NULL, '{}'),
    ('team-c', 'org-2', '{}', '{}', '{}', '[]', 1000, 1000, '{"model_rpm_limit": {"gpt": 1000}}');
"#;
const TEAM_A_TPM_SQL: &str =
    r#"SELECT tpm_limit FROM "LiteLLM_TeamTable" WHERE team_id = 'team-a'"#;
const TEAM_COUNT_SQL: &str = r#"SELECT count(*) FROM "LiteLLM_TeamTable""#;

#[tokio::test]
async fn siblings_include_updated_team() {
    let (fixture, store) = database(SIBLINGS).await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };
    let budget = r#"{"tpm_limit": 100, "rpm_limit": 10}"#;
    let mut organization = org("[]", budget);
    organization.metadata =
        Some(serde_json::from_str(r#"{"model_rpm_limit": {"gpt": 5}}"#).unwrap());
    let guaranteed = r#""metadata": {"tpm_limit_type": "guaranteed_throughput"}"#;
    let over = update_team(&format!(
        r#"{{"team_id": "team-a", "tpm_limit": 21, {guaranteed}}}"#
    ));
    let rejection = rejected(
        check_org_team_limits(&organization, &TeamLimitRequest::from(&over), Some(client)).await,
    );
    assert_eq!(
        allocation_text(&rejection),
        "Allocated TPM limit=80 + Team TPM limit=21 is greater than organization TPM limit=100"
    );
    let fits = update_team(&format!(
        r#"{{"team_id": "team-a", "tpm_limit": 20, {guaranteed}}}"#
    ));
    check_org_team_limits(&organization, &TeamLimitRequest::from(&fits), Some(client))
        .await
        .unwrap();
    let model = update_team(&format!(
        r#"{{"team_id": "team-a", "model_rpm_limit": {{"gpt": 3}}, {guaranteed}}}"#
    ));
    let per_model = rejected(
        check_org_team_limits(&organization, &TeamLimitRequest::from(&model), Some(client)).await,
    );
    assert_eq!(
        allocation_text(&per_model),
        "Allocated RPM limit=3 + Team RPM limit=3 is greater than organization RPM limit=5"
    );
    assert_eq!(count(&store, TEAM_COUNT_SQL).await, 3);
    assert_eq!(count(&store, TEAM_A_TPM_SQL).await, 50);
    close(fixture, store).await;
}
