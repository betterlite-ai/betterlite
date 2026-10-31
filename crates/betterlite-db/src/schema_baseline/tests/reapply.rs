#[tokio::test]
async fn baseline_reapply() {
    harness(|[client]| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        client.batch_execute(migration_sql()).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn baseline_reapply_rows_preserved() {
    harness(|[client]| async move {
        client.batch_execute(migration_sql()).await.unwrap();
        insert_budget(&client, "b1").await;
        exec(
            &client,
            r#"INSERT INTO "LiteLLM_CredentialsTable" (credential_id, credential_name, credential_values, created_by, updated_by) VALUES ('c1', 'n', '{"api_key":"sk-raw-secret"}', 't', 't')"#,
        )
        .await;
        client.batch_execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, password) VALUES ('u1', 'raw-password');
            INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ('raw-token');"#,
        ).await.unwrap();
        client.batch_execute(migration_sql()).await.unwrap();
        let budget_count = scalar_i64(
            &client,
            r#"SELECT count(*) FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b1'"#,
        )
        .await;
        assert_eq!(budget_count, 1);
        let credential_values = scalar_text(
            &client,
            r#"SELECT credential_values::TEXT FROM "LiteLLM_CredentialsTable" WHERE credential_id = 'c1'"#,
        )
        .await;
        assert!(credential_values.contains("sk-raw-secret"));
        let password = scalar_text(
            &client,
            r#"SELECT password FROM "LiteLLM_UserTable" WHERE user_id = 'u1'"#,
        )
        .await;
        assert_eq!(password, "raw-password");
        let token = scalar_text(&client, r#"SELECT token FROM "LiteLLM_VerificationToken""#).await;
        assert_eq!(token, "raw-token");
    })
    .await;
}

#[tokio::test]
async fn baseline_reapply_drifted_existing_table_not_repaired() {
    harness(|[client]| async move {
        client
            .batch_execute(
                r#"CREATE TABLE IF NOT EXISTS "LiteLLM_BudgetTable" ("budget_id" TEXT NOT NULL, CONSTRAINT "LiteLLM_BudgetTable_pkey" PRIMARY KEY ("budget_id"));"#,
            )
            .await
            .unwrap();
        client.batch_execute(migration_sql()).await.unwrap();
        let exists: bool = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'LiteLLM_BudgetTable' AND column_name = 'max_budget')",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!exists);
    })
    .await;
}

#[tokio::test]
async fn baseline_reapply_foreign_key_name_in_other_schema_suppresses_creation() {
    harness(|[client]| async move {
        client
            .batch_execute(
                r#"CREATE SCHEMA other_schema;
               CREATE TABLE other_schema.dummy (id INT);
               ALTER TABLE other_schema.dummy ADD CONSTRAINT "LiteLLM_OrganizationTable_budget_id_fkey" CHECK (true);"#,
            )
            .await
            .unwrap();
        client.batch_execute(migration_sql()).await.unwrap();
        let real_fk_count: i64 = client
            .query_one(
                r#"SELECT count(*) FROM pg_constraint WHERE conrelid = '"LiteLLM_OrganizationTable"'::regclass AND contype = 'f'"#,
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(real_fk_count, 0);
    })
    .await;
}

#[tokio::test]
async fn baseline_reapply_independent_databases_do_not_collide() {
    Box::pin(harness(|[client_a, client_b]| async move {
        client_a.batch_execute(migration_sql()).await.unwrap();
        client_a
            .batch_execute(
                r#"ALTER TABLE "LiteLLM_OrganizationTable" DROP CONSTRAINT "LiteLLM_OrganizationTable_budget_id_fkey";
               CREATE SCHEMA other_schema;
               CREATE TABLE other_schema.dummy (id INT);
               ALTER TABLE other_schema.dummy ADD CONSTRAINT "LiteLLM_OrganizationTable_budget_id_fkey" CHECK (true);"#,
            )
            .await
            .unwrap();
        client_a.batch_execute(migration_sql()).await.unwrap();

        client_b.batch_execute(migration_sql()).await.unwrap();

        let fk_query = r#"SELECT count(*) FROM pg_constraint WHERE conrelid = '"LiteLLM_OrganizationTable"'::regclass AND contype = 'f'"#;
        let fk_in_a: i64 = client_a.query_one(fk_query, &[]).await.unwrap().get(0);
        let fk_in_b: i64 = client_b.query_one(fk_query, &[]).await.unwrap().get(0);
        assert_eq!(fk_in_a, 0);
        assert_eq!(fk_in_b, 1);
    }))
    .await;
}
