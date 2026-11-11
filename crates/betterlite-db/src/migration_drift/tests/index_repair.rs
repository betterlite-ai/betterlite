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
        let captured = logs.joined();
        assert!(captured.contains("found invalid indexes"), "{captured}");
        assert!(captured.contains("Dropped leftover"), "{captured}");
        assert!(captured.contains("Rebuilt"), "{captured}");
        assert!(
            invalid_litellm_indexes(raw, &database.schema)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            index_valid(raw, &database.schema, "LiteLLM_aaa_repairable").await,
            Some(true)
        );
        assert_eq!(
            index_valid(raw, &database.schema, "LiteLLM_ccc_leftover_ccnew1").await,
            None
        );
        assert_eq!(
            index_valid(raw, &database.schema, "other_idx").await,
            Some(false)
        );
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
