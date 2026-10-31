use rstest::rstest;
use tokio_postgres::error::SqlState;

struct FkCase {
    conname: &'static str,
    delete_rule: char,
    setup_sql: &'static str,
    update_sql: &'static str,
    read_sql: &'static str,
    delete_sql: &'static str,
    expected_after_update: &'static str,
}

#[rstest]
#[case::organization_table_budget_id_fkey(FkCase {
    conname: "LiteLLM_OrganizationTable_budget_id_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'b2' WHERE budget_id = 'b1';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b2';"#,
    expected_after_update: "b2",
})]
#[case::team_table_organization_id_fkey(FkCase {
    conname: "LiteLLM_TeamTable_organization_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_TeamTable" (team_id, organization_id) VALUES ('t1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_OrganizationTable" SET organization_id = 'o2' WHERE organization_id = 'o1';"#,
    read_sql: r#"SELECT organization_id FROM "LiteLLM_TeamTable" WHERE team_id = 't1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o2';"#,
    expected_after_update: "o2",
})]
#[case::team_table_model_id_fkey(FkCase {
    conname: "LiteLLM_TeamTable_model_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_ModelTable" (id, created_by, updated_by) VALUES (9001, 't', 't');
       INSERT INTO "LiteLLM_TeamTable" (team_id, model_id) VALUES ('t1', 9001);"#,
    update_sql: r#"UPDATE "LiteLLM_ModelTable" SET id = 9002 WHERE id = 9001;"#,
    read_sql: r#"SELECT model_id::TEXT FROM "LiteLLM_TeamTable" WHERE team_id = 't1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_ModelTable" WHERE id = 9002;"#,
    expected_after_update: "9002",
})]
#[case::user_table_organization_id_fkey(FkCase {
    conname: "LiteLLM_UserTable_organization_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_UserTable" (user_id, organization_id) VALUES ('u1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_OrganizationTable" SET organization_id = 'o2' WHERE organization_id = 'o1';"#,
    read_sql: r#"SELECT organization_id FROM "LiteLLM_UserTable" WHERE user_id = 'u1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o2';"#,
    expected_after_update: "o2",
})]
#[case::verification_token_budget_id_fkey(FkCase {
    conname: "LiteLLM_VerificationToken_budget_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_VerificationToken" (token, budget_id) VALUES ('tok1', 'b1');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'b2' WHERE budget_id = 'b1';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_VerificationToken" WHERE token = 'tok1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b2';"#,
    expected_after_update: "b2",
})]
#[case::verification_token_organization_id_fkey(FkCase {
    conname: "LiteLLM_VerificationToken_organization_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_VerificationToken" (token, organization_id) VALUES ('tok1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_OrganizationTable" SET organization_id = 'o2' WHERE organization_id = 'o1';"#,
    read_sql: r#"SELECT organization_id FROM "LiteLLM_VerificationToken" WHERE token = 'tok1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o2';"#,
    expected_after_update: "o2",
})]
#[case::end_user_table_budget_id_fkey(FkCase {
    conname: "LiteLLM_EndUserTable_budget_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_EndUserTable" (user_id, budget_id) VALUES ('eu1', 'b1');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'b2' WHERE budget_id = 'b1';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_EndUserTable" WHERE user_id = 'eu1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b2';"#,
    expected_after_update: "b2",
})]
#[case::team_membership_budget_id_fkey(FkCase {
    conname: "LiteLLM_TeamMembership_budget_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, budget_id) VALUES ('u1', 't1', 'b1');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'b2' WHERE budget_id = 'b1';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_TeamMembership" WHERE user_id = 'u1' AND team_id = 't1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b2';"#,
    expected_after_update: "b2",
})]
#[case::organization_membership_user_id_fkey(FkCase {
    conname: "LiteLLM_OrganizationMembership_user_id_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id) VALUES ('u1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_UserTable" SET user_id = 'u2' WHERE user_id = 'u1';"#,
    read_sql: r#"SELECT user_id FROM "LiteLLM_OrganizationMembership" WHERE organization_id = 'o1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_UserTable" WHERE user_id = 'u2';"#,
    expected_after_update: "u2",
})]
#[case::organization_membership_organization_id_fkey(FkCase {
    conname: "LiteLLM_OrganizationMembership_organization_id_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id) VALUES ('u1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_OrganizationTable" SET organization_id = 'o2' WHERE organization_id = 'o1';"#,
    read_sql: r#"SELECT organization_id FROM "LiteLLM_OrganizationMembership" WHERE user_id = 'u1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o2';"#,
    expected_after_update: "o2",
})]
#[case::organization_membership_budget_id_fkey(FkCase {
    conname: "LiteLLM_OrganizationMembership_budget_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('bOrg', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'bOrg', 't', 't');
       INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('bM', 't', 't');
       INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id, budget_id) VALUES ('u1', 'o1', 'bM');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'bM2' WHERE budget_id = 'bM';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_OrganizationMembership" WHERE user_id = 'u1' AND organization_id = 'o1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'bM2';"#,
    expected_after_update: "bM2",
})]
#[case::invitation_link_user_id_fkey(FkCase {
    conname: "LiteLLM_InvitationLink_user_id_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u2');
       INSERT INTO "LiteLLM_InvitationLink" (id, user_id, expires_at, created_at, created_by, updated_at, updated_by) VALUES ('i1', 'u1', now(), now(), 'u2', now(), 'u2');"#,
    update_sql: r#"UPDATE "LiteLLM_UserTable" SET user_id = 'u1x' WHERE user_id = 'u1';"#,
    read_sql: r#"SELECT user_id FROM "LiteLLM_InvitationLink" WHERE id = 'i1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_UserTable" WHERE user_id = 'u1x';"#,
    expected_after_update: "u1x",
})]
#[case::invitation_link_created_by_fkey(FkCase {
    conname: "LiteLLM_InvitationLink_created_by_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u2');
       INSERT INTO "LiteLLM_InvitationLink" (id, user_id, expires_at, created_at, created_by, updated_at, updated_by) VALUES ('i1', 'u2', now(), now(), 'u1', now(), 'u2');"#,
    update_sql: r#"UPDATE "LiteLLM_UserTable" SET user_id = 'u1x' WHERE user_id = 'u1';"#,
    read_sql: r#"SELECT created_by FROM "LiteLLM_InvitationLink" WHERE id = 'i1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_UserTable" WHERE user_id = 'u1x';"#,
    expected_after_update: "u1x",
})]
#[case::invitation_link_updated_by_fkey(FkCase {
    conname: "LiteLLM_InvitationLink_updated_by_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u2');
       INSERT INTO "LiteLLM_InvitationLink" (id, user_id, expires_at, created_at, created_by, updated_at, updated_by) VALUES ('i1', 'u2', now(), now(), 'u2', now(), 'u1');"#,
    update_sql: r#"UPDATE "LiteLLM_UserTable" SET user_id = 'u1x' WHERE user_id = 'u1';"#,
    read_sql: r#"SELECT updated_by FROM "LiteLLM_InvitationLink" WHERE id = 'i1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_UserTable" WHERE user_id = 'u1x';"#,
    expected_after_update: "u1x",
})]
#[tokio::test]
async fn baseline_foreign_keys(#[case] case: FkCase) {
    harness(move |[client]| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        client.batch_execute(case.setup_sql).await.unwrap();
        assert_rule(&client, case.conname, case.delete_rule).await;
        client.batch_execute(case.update_sql).await.unwrap();
        let after_update: String = client.query_one(case.read_sql, &[]).await.unwrap().get(0);
        assert_eq!(after_update, case.expected_after_update);
        if case.delete_rule == 'r' {
            let error = client.batch_execute(case.delete_sql).await.unwrap_err();
            assert_eq!(error.code(), Some(&SqlState::FOREIGN_KEY_VIOLATION));
        } else {
            client.batch_execute(case.delete_sql).await.unwrap();
            let after_delete: Option<String> =
                client.query_one(case.read_sql, &[]).await.unwrap().get(0);
            assert_eq!(after_delete, None);
        }
    })
    .await;
}
