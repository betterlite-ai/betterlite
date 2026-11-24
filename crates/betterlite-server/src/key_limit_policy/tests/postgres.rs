use crate::key_fields::shared::hash_token_if_needed;
use crate::key_limit_policy::checks::{
    check_project_key_limits, check_team_key_limits, validate_org_assignment,
};
use crate::key_limit_policy::error::Rejection;
use crate::key_limit_policy::request::KeyLimitRequest;

use super::support::{close, database, generate, reads, rejected, team, text_detail, update};

#[tokio::test]
async fn team_query_excludes_current() {
    let hashed = hash_token_if_needed("sk-abc");
    let seed = format!(
        r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "team_id", "rpm_limit", "models") VALUES ('{hashed}', 'team-1', 10, '{{}}'), ('other-team-1', 'team-1', 0, '{{}}'), ('other-team-2', 'team-2', 10, '{{}}');"#
    );
    let (fixture, store) = database(&seed).await;
    let recorded = reads(&store);
    let team_table = team(r#"{"team_id": "team-1", "rpm_limit": 10}"#);
    let request =
        update(r#"{"key": "sk-abc", "rpm_limit_type": "guaranteed_throughput", "rpm_limit": 10}"#);
    check_team_key_limits(&team_table, &KeyLimitRequest::from(&request), &recorded)
        .await
        .unwrap();
    let client = store.session().await.unwrap();
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "team_id", "rpm_limit", "models") VALUES ('extra', 'team-1', 5, '{}');"#,
        )
        .await
        .unwrap();
    let rejection = rejected(
        check_team_key_limits(&team_table, &KeyLimitRequest::from(&request), &recorded).await,
    );
    assert_eq!(
        text_detail(&rejection),
        "Allocated RPM limit=5 + Key RPM limit=10 is greater than team RPM limit=10"
    );
    close(fixture, store).await;
}

const MEMBERSHIP_SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" ("budget_id", "created_by", "updated_by") VALUES ('budget-1', 'admin', 'admin');
INSERT INTO "LiteLLM_OrganizationTable" ("organization_id", "organization_alias", "budget_id", "models", "created_by", "updated_by") VALUES ('org-1', 'org-1', 'budget-1', '{}', 'admin', 'admin');
INSERT INTO "LiteLLM_UserTable" ("user_id", "models") VALUES ('u1', '{}'), ('u2', '{}');
INSERT INTO "LiteLLM_OrganizationMembership" ("user_id", "organization_id", "user_role") VALUES ('u1', 'org-1', 'internal_user');
"#;

#[tokio::test]
async fn org_membership_query() {
    let (fixture, store) = database(MEMBERSHIP_SEED).await;
    let recorded = reads(&store);
    validate_org_assignment(Some("u1"), "org-1", &recorded)
        .await
        .unwrap();
    let rejection = rejected(validate_org_assignment(Some("u1"), "org-2", &recorded).await);
    assert_eq!(
        text_detail(&rejection),
        "Caller is not a member of organization_id=org-2"
    );
    let not_member = rejected(validate_org_assignment(Some("u2"), "org-1", &recorded).await);
    assert_eq!(
        text_detail(&not_member),
        "Caller is not a member of organization_id=org-1"
    );
    close(fixture, store).await;
}

const PROJECT_SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" ("budget_id", "max_budget", "created_by", "updated_by") VALUES ('budget-1', 10, 'admin', 'admin');
INSERT INTO "LiteLLM_ProjectTable" ("project_id", "budget_id", "models", "created_by", "updated_by") VALUES ('proj-1', 'budget-1', '{gpt-4}', 'admin', 'admin');
"#;

#[tokio::test]
async fn project_query_reads_the_row() {
    let (fixture, store) = database(PROJECT_SEED).await;
    let recorded = reads(&store);
    let model = generate(r#"{"models": ["claude-4"]}"#);
    let model_request = KeyLimitRequest::from(&model);
    let checked = check_project_key_limits("proj-1", &model_request, &recorded);
    match rejected(checked.await) {
        Rejection::ProjectModel { allowed, .. } => assert_eq!(allowed, vec!["gpt-4".to_owned()]),
        other @ (Rejection::NonFinite { .. }
        | Rejection::Upperbound { .. }
        | Rejection::Allocation { .. }
        | Rejection::ProjectNotFound { .. }
        | Rejection::ProjectBudget { .. }
        | Rejection::AssignWithoutUser
        | Rejection::NotMember { .. }) => {
            panic!("expected the stored model list to refuse claude-4, got {other:?}")
        }
    }
    let budget = generate(r#"{"models": ["gpt-4"], "max_budget": 11}"#);
    let budget_request = KeyLimitRequest::from(&budget);
    let budget_checked = check_project_key_limits("proj-1", &budget_request, &recorded);
    assert!(matches!(
        rejected(budget_checked.await),
        Rejection::ProjectBudget { .. }
    ));
    let within = generate(r#"{"models": ["gpt-4"], "max_budget": 10}"#);
    let within_request = KeyLimitRequest::from(&within);
    check_project_key_limits("proj-1", &within_request, &recorded)
        .await
        .unwrap();
    let missing = check_project_key_limits("proj-2", &within_request, &recorded);
    assert!(matches!(
        rejected(missing.await),
        Rejection::ProjectNotFound { .. }
    ));
    close(fixture, store).await;
}
