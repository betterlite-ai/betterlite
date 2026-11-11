#[tokio::test]
async fn invalid_index_repair() {
    Box::pin(with_schema(|database| async move {
        let client = database.pool.get().await.unwrap();
        let raw = &**client;
        create_repairable_indexes(raw).await;
        let found = invalid_litellm_indexes(raw, &database.schema)
            .await
            .unwrap();
        assert_eq!(
            index_names(&found),
            ["LiteLLM_aaa_repairable", "LiteLLM_ccc_leftover_ccnew1"]
        );

        let (logs, dispatch) = capture();
        let _guard = set_default(&dispatch);
        assert!(repair_invalid_indexes(&database.check(), "30s").await);
        assert_repair_result(raw, &database.schema, &logs.joined()).await;
        assert!(repair_invalid_indexes(&database.check(), "30s").await);
    }))
    .await;
}

#[tokio::test]
async fn repair_without_database_returns_false() {
    let absent = MigrationDriftCheck {
        database_url: None,
        direct_url: None,
    };
    assert!(!repair_invalid_indexes(&absent, "30s").await);
    let malformed = MigrationDriftCheck {
        database_url: Some(SecretString::from("postgresql://[::1/db")),
        direct_url: None,
    };
    assert!(!repair_invalid_indexes(&malformed, "30s").await);
    let unreachable = MigrationDriftCheck {
        database_url: Some(SecretString::from(
            "postgresql://postgres:postgres@127.0.0.1:1/postgres",
        )),
        direct_url: None,
    };
    assert!(!repair_invalid_indexes(&unreachable, "30s").await);
}

#[tokio::test]
async fn repair_skips_an_index_that_vanished() {
    Box::pin(with_schema(|database| async move {
        let client = database.pool.get().await.unwrap();
        let raw = &**client;
        duplicate_table(raw, "LiteLLM_Stuck").await;
        invalid_unique_index(raw, "LiteLLM_Stuck", "LiteLLM_zzz_stuck").await;
        let (logs, dispatch) = capture();
        let _guard = set_default(&dispatch);
        let vanished = InvalidIndex {
            schema: database.schema.clone(),
            name: "LiteLLM_gone".to_owned(),
            table_size: String::new(),
        };
        let repaired = repair_under_migration_lock(raw, &database.schema, &vanished).await;
        assert!(repaired.unwrap());
        assert!(logs.joined().is_empty(), "{}", logs.joined());
    }))
    .await;
}

#[test]
fn index_repair_sql_quotes_identifiers() {
    let index = InvalidIndex {
        schema: "tenant\"a".to_owned(),
        name: "LiteLLM_idx".to_owned(),
        table_size: String::new(),
    };
    let (statement, action) = index_repair(&index);
    assert_eq!(
        statement,
        "REINDEX INDEX CONCURRENTLY \"tenant\"\"a\".\"LiteLLM_idx\""
    );
    assert_eq!(action, "Rebuilt");
}

#[rstest]
#[case::ccnew("LiteLLM_a_ccnew", true)]
#[case::ccnew_number("LiteLLM_a_ccnew12", true)]
#[case::ccold("LiteLLM_a_ccold1", true)]
#[case::embedded("LiteLLM_a_ccnew1_more", false)]
#[case::plain("LiteLLM_a", false)]
fn transient_suffix_selects_drop(#[case] name: &str, #[case] transient: bool) {
    let index = InvalidIndex {
        schema: "public".to_owned(),
        name: name.to_owned(),
        table_size: String::new(),
    };
    let (statement, action) = index_repair(&index);
    assert_eq!(
        statement.starts_with("DROP INDEX CONCURRENTLY IF EXISTS "),
        transient
    );
    assert_eq!(
        action,
        if transient {
            "Dropped leftover"
        } else {
            "Rebuilt"
        }
    );
}

#[test]
fn repair_url_prefers_direct() {
    let database = SecretString::from(
        "postgresql://writer/db?schema=tenant_a&sslmode=require&connection_limit=3",
    );
    let direct = SecretString::from("postgresql://direct:6543/db?pgbouncer=true");
    let chosen = repair_connection_url(Some(&database), Some(&direct))
        .unwrap()
        .unwrap();
    assert_eq!(chosen, "postgresql://direct:6543/db");
    let fallback = repair_connection_url(Some(&database), None)
        .unwrap()
        .unwrap();
    assert_eq!(fallback, "postgresql://writer/db?sslmode=require");
    assert_eq!(repair_connection_url(None, None).unwrap(), None);
    let malformed = SecretString::from("postgresql://[::1/db");
    repair_connection_url(Some(&malformed), None).unwrap_err();
}
