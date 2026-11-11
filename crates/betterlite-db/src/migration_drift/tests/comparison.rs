#[tokio::test]
async fn disabled_updates_difference_warns() {
    Box::pin(with_schema(|database| async move {
        let client = database.pool.get().await.unwrap();
        client
            .batch_execute("CREATE TABLE \"LiteLLM_Drift\" (id text PRIMARY KEY, value text)")
            .await
            .unwrap();
        let expected = read_contract(&database.pool, &database.schema).await;
        client
            .batch_execute("ALTER TABLE \"LiteLLM_Drift\" DROP COLUMN value")
            .await
            .unwrap();
        let (logs, dispatch) = capture();
        let _guard = set_default(&dispatch);
        let outcome = schema_check(&database, &expected, true).await;
        let SchemaCheck::Differing(comparison) = outcome else {
            panic!("a dropped column must be reported as drift");
        };
        assert!(changed(
            &comparison,
            ObjectKind::Column,
            "LiteLLM_Drift",
            "value",
            Presence::Missing
        ));
        assert!(logs.joined().contains("out of sync"));

        let before = logs.joined();
        let enabled = schema_check(&database, &expected, false).await;
        assert_eq!(enabled, SchemaCheck::UpdatesEnabled);
        assert_eq!(logs.joined(), before);
    }))
    .await;
}

#[tokio::test]
async fn comparison_failure_warns() {
    let expected = base_contract();
    let (logs, dispatch) = capture();
    let _guard = set_default(&dispatch);
    let missing = MigrationDriftCheck {
        database_url: None,
        direct_url: None,
    };
    assert_eq!(
        check_schema(&missing, true, Some(&expected), PUBLIC_SCHEMA, READ_TIMEOUT).await,
        SchemaCheck::Unavailable
    );
    assert!(logs.joined().contains("DATABASE_URL is not set"));

    let malformed = MigrationDriftCheck {
        database_url: Some(SecretString::from("postgresql://[::1/db")),
        direct_url: None,
    };
    assert_eq!(
        check_schema(
            &malformed,
            true,
            Some(&expected),
            PUBLIC_SCHEMA,
            READ_TIMEOUT
        )
        .await,
        SchemaCheck::Unavailable
    );
    assert!(logs.joined().contains("could not be compared"));
    assert_eq!(
        check_schema(&malformed, true, None, PUBLIC_SCHEMA, READ_TIMEOUT).await,
        SchemaCheck::Unavailable
    );
    assert!(logs.joined().contains("schema contract is unavailable"));
}

#[test]
fn difference_summary_names_facts() {
    let expected = contract(vec![
        fact(ObjectKind::Relation, "LiteLLM_Base", "", Attribute::Kind),
        fact(
            ObjectKind::Column,
            "LiteLLM_Base",
            "removed",
            Attribute::Type,
        ),
    ]);
    let live = contract(vec![
        fact(ObjectKind::Relation, "LiteLLM_Base", "", Attribute::Kind),
        fact(
            ObjectKind::Index,
            "LiteLLM_Base",
            "extra_idx",
            Attribute::Method,
        ),
    ]);
    let comparison = expected.compare(&live);
    assert_eq!(comparison.differences.len(), 2);
    let rendered = summary(&comparison);
    assert!(rendered.contains("LiteLLM_Base.removed"), "{rendered}");
    assert!(rendered.contains("LiteLLM_Base.extra_idx"), "{rendered}");
}

#[test]
fn difference_summary_is_bounded() {
    let expected = base_contract();
    let extras: Vec<Fact> = (0_u8..7)
        .map(|index| {
            fact(
                ObjectKind::Relation,
                &format!("LiteLLM_Extra_{index}"),
                "",
                Attribute::Kind,
            )
        })
        .collect();
    let live = contract(extras);
    let (logs, dispatch) = capture();
    let _guard = set_default(&dispatch);
    let SchemaCheck::Differing(comparison) = evaluate(&expected, &live) else {
        panic!("extra relations must be reported");
    };
    assert_eq!(comparison.differences.len(), 8);
    let rendered = summary(&comparison);
    assert!(rendered.contains("and 3 more"), "{rendered}");
    assert!(logs.joined().contains("out of sync"));
    assert_eq!(evaluate(&expected, &expected), SchemaCheck::InSync);
}
