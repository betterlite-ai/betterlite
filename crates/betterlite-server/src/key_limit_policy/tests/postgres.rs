use crate::key_fields::shared::hash_token_if_needed;
use crate::key_limit_policy::checks::{check_team_key_limits, validate_org_assignment};
use crate::key_limit_policy::request::KeyLimitRequest;

use super::support::{close, database, reads, rejected, team, text_detail, update};

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
    let rejection = rejected(validate_org_assignment(Some("u2"), "org-1", &recorded).await);
    assert_eq!(
        text_detail(&rejection),
        "Caller is not a member of organization_id=org-1"
    );
    close(fixture, store).await;
}
