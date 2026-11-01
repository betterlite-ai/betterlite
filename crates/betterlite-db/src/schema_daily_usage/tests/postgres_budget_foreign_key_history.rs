const ADD_DAILY_USER_TABLE: &str = "20250326171002_add_daily_user_table";
const CASCADE_BUDGET_CHANGES: &str = "20250603210143_cascade_budget_changes";
const CASCADE_BUDGET_AND_LOOSEN_MANAGED_FILE_JSON: &str =
    "20250625145206_cascade_budget_and_loosen_managed_file_json";

async fn seed_membership(client: &Client, user_id: &str, team_id: &str, budget_id: &str) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ($1, 't', 't')"#,
        &[&budget_id],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ($1)"#,
        &[&team_id],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, budget_id) VALUES ($1, $2, $3)"#,
        &[&user_id, &team_id, &budget_id],
    )
    .await;
}

async fn remove_budget(client: &Client, budget_id: &str) {
    exec_params(
        client,
        r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = $1"#,
        &[&budget_id],
    )
    .await;
}

#[tokio::test]
async fn intermediate_budget_delete_cascades_membership() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(CASCADE_BUDGET_CHANGES)).await;
        seed_membership(&client, "u1", "t1", "b1").await;
        remove_budget(&client, "b1").await;
        assert!(!membership_exists(&client, "u1", "t1").await);
    })
    .await;
}

#[tokio::test]
async fn final_budget_delete_sets_membership_null() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through(CASCADE_BUDGET_AND_LOOSEN_MANAGED_FILE_JSON),
        )
        .await;
        seed_membership(&client, "u1", "t1", "b1").await;
        remove_budget(&client, "b1").await;
        assert!(membership_exists(&client, "u1", "t1").await);
        assert_eq!(membership_budget_id(&client, "u1", "t1").await, None);
    })
    .await;
}

#[tokio::test]
async fn budget_id_update_cascades() {
    harness(|[client]| async move {
        batch_execute(
            &client,
            &migration_sql_through(CASCADE_BUDGET_AND_LOOSEN_MANAGED_FILE_JSON),
        )
        .await;
        seed_membership(&client, "u1", "t1", "b1").await;
        exec_params(
            &client,
            r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = $1 WHERE budget_id = $2"#,
            &[&"b2", &"b1"],
        )
        .await;
        assert_eq!(
            membership_budget_id(&client, "u1", "t1").await,
            Some("b2".to_owned())
        );
    })
    .await;
}

#[tokio::test]
async fn prior_rows_unchanged_by_constraint_replacement() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(ADD_DAILY_USER_TABLE)).await;
        seed_membership(&client, "u1", "t1", "b1").await;
        batch_execute(
            &client,
            &migration_sql_between(ADD_DAILY_USER_TABLE, CASCADE_BUDGET_CHANGES),
        )
        .await;
        assert_eq!(
            membership_budget_id(&client, "u1", "t1").await,
            Some("b1".to_owned())
        );
        batch_execute(
            &client,
            &migration_sql_between(
                CASCADE_BUDGET_CHANGES,
                CASCADE_BUDGET_AND_LOOSEN_MANAGED_FILE_JSON,
            ),
        )
        .await;
        assert_eq!(
            membership_budget_id(&client, "u1", "t1").await,
            Some("b1".to_owned())
        );
    })
    .await;
}

#[tokio::test]
async fn other_schema_constraint_drop_42704() {
    harness(|[client]| async move {
        batch_execute(&client, &migration_sql_through(ADD_DAILY_USER_TABLE)).await;
        batch_execute(
            &client,
            r#"ALTER TABLE "LiteLLM_TeamMembership" DROP CONSTRAINT "LiteLLM_TeamMembership_budget_id_fkey";"#,
        )
        .await;
        create_decoy_constraint(&client, "LiteLLM_TeamMembership_budget_id_fkey").await;
        let error = batch_execute_err(&client, migration_sql_of(CASCADE_BUDGET_CHANGES)).await;
        assert_eq!(error.code().unwrap().code(), "42704");
    })
    .await;
}
