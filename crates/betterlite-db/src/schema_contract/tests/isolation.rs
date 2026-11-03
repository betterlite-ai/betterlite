use tokio_postgres::error::SqlState;
#[tokio::test]
async fn postgres_read_only_comparison() {
    harness(|[reference, candidate]| async move {
        let expected = snapshot(&reference).await;
        let role = format!("contract_{}", uuid::Uuid::new_v4().simple());
        candidate
            .batch_execute(&format!("CREATE ROLE {role} NOLOGIN; SET ROLE {role}"))
            .await
            .unwrap();
        let outcome = AssertUnwindSafe(async {
            let denied = candidate
                .batch_execute("CREATE TABLE public.forbidden (id int)")
                .await
                .unwrap_err();
            assert_eq!(denied.code(), Some(&SqlState::INSUFFICIENT_PRIVILEGE));
            candidate.batch_execute("BEGIN READ ONLY").await.unwrap();
            let actual = snapshot(&candidate).await;
            assert!(expected.compare(&actual).differences.is_empty());
            candidate.batch_execute("ROLLBACK").await.unwrap();
        })
        .catch_unwind()
        .await;
        candidate
            .batch_execute(&format!("ROLLBACK; RESET ROLE; DROP ROLE {role}"))
            .await
            .unwrap();
        if let Err(payload) = outcome {
            resume_unwind(payload);
        }
    })
    .await;
}

#[tokio::test]
async fn quoted_namespace() {
    harness(|[reference, candidate]| async move {
        let expected = snapshot(&reference).await;
        candidate
            .batch_execute("ALTER SCHEMA public RENAME TO \"Odd ' \"\"; schema\"")
            .await
            .unwrap();
        let actual = SchemaContract::read(&**candidate, "Odd ' \"; schema", LIMIT)
            .await
            .unwrap();
        assert!(
            expected.compare(&actual).differences.is_empty(),
            "{:#?}",
            expected.compare(&actual)
        );
    })
    .await;
}

#[tokio::test]
async fn unrelated_namespace_ignored() {
    harness(|[reference, candidate]| async move {
        let expected = snapshot(&reference).await;
        candidate
            .batch_execute(
                "CREATE SCHEMA unrelated; CREATE TABLE unrelated.\"LiteLLM_Lens\" (bad boolean)",
            )
            .await
            .unwrap();
        assert!(
            expected
                .compare(&snapshot(&candidate).await)
                .differences
                .is_empty()
        );
    })
    .await;
}

#[tokio::test]
async fn extra_relations_reported_without_drop() {
    harness(|[reference, candidate]| async move {
        candidate
            .batch_execute(
                "CREATE TABLE operator_data (id int); INSERT INTO operator_data VALUES (42)",
            )
            .await
            .unwrap();
        let report = snapshot(&reference)
            .await
            .compare(&snapshot(&candidate).await);
        assert!(
            report
                .differences
                .iter()
                .any(|(fact, change)| fact.kind == ObjectKind::Relation
                    && fact.relation == "operator_data"
                    && *change
                        == Change::Extra {
                            actual: Some("r".to_owned())
                        })
        );
        assert_eq!(
            candidate
                .query_one("SELECT id FROM operator_data", &[])
                .await
                .unwrap()
                .get::<_, i32>(0),
            42_i32
        );
    })
    .await;
}

#[tokio::test]
async fn different_oids_same_contract() {
    harness(|[reference, candidate]| async move {
        let query = "SELECT 'public.\"LiteLLM_Lens\"'::regclass::oid";
        let first: u32 = reference.query_one(query, &[]).await.unwrap().get(0);
        let second: u32 = candidate.query_one(query, &[]).await.unwrap().get(0);
        assert_ne!(first, second);
        assert!(
            snapshot(&reference)
                .await
                .compare(&snapshot(&candidate).await)
                .differences
                .is_empty()
        );
    })
    .await;
}

#[tokio::test]
async fn existing_rows_and_sequence_unchanged() {
    harness(|[reference, candidate]| async move {
        candidate.batch_execute("INSERT INTO \"LiteLLM_UserTable\" (user_id) VALUES ('user'); INSERT INTO \"LiteLLM_MCPUserCredentials\" (id,user_id,server_id,credential_b64) VALUES ('secret','user','server','encoded'); CREATE TABLE operator_ledger (entry text); INSERT INTO operator_ledger VALUES ('ledger'); SELECT nextval('\"LiteLLM_ModelTable_id_seq\"')").await.unwrap();
        let sql = "SELECT (SELECT row_to_json(u)::text FROM \"LiteLLM_UserTable\" u), (SELECT row_to_json(c)::text FROM \"LiteLLM_MCPUserCredentials\" c), (SELECT entry FROM operator_ledger), last_value, is_called FROM \"LiteLLM_ModelTable_id_seq\"";
        let before = candidate.query_one(sql, &[]).await.unwrap();
        let expected = snapshot(&reference).await;
        let actual = snapshot(&candidate).await;
        let report = expected.compare(&actual);
        assert!(report.differences.keys().all(|fact| fact.relation == "operator_ledger"));
        let after = candidate.query_one(sql, &[]).await.unwrap();
        assert_eq!(before.get::<_, String>(0), after.get::<_, String>(0));
        assert_eq!(before.get::<_, String>(1), after.get::<_, String>(1));
        assert_eq!(before.get::<_, String>(2), after.get::<_, String>(2));
        assert_eq!(before.get::<_, i64>(3), after.get::<_, i64>(3));
        assert_eq!(before.get::<_, bool>(4), after.get::<_, bool>(4));
    }).await;
}

#[tokio::test]
async fn absent_namespace_is_an_error() {
    harness(|[_reference, candidate]| async move {
        let error = SchemaContract::read(&**candidate, "missing", LIMIT)
            .await
            .unwrap_err();
        assert!(matches!(error, super::ContractError::Namespace(name) if name == "missing"));
    })
    .await;
}
