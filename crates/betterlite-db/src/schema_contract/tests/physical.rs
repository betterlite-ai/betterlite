#[rstest]
#[case::array("ALTER TABLE \"LiteLLM_TeamTable\" ALTER COLUMN models SET NOT NULL", (ObjectKind::Column, "LiteLLM_TeamTable", "models", "not_null"), ("false", "true"))]
#[case::method("DROP INDEX \"LiteLLM_MemoryTable_user_id_idx\"; CREATE INDEX \"LiteLLM_MemoryTable_user_id_idx\" ON \"LiteLLM_MemoryTable\" USING hash(user_id)", (ObjectKind::Index, "LiteLLM_MemoryTable", "LiteLLM_MemoryTable_user_id_idx", "method"), ("btree", "hash"))]
#[case::uniqueness("DROP INDEX \"LiteLLM_MemoryTable_user_id_idx\"; CREATE UNIQUE INDEX \"LiteLLM_MemoryTable_user_id_idx\" ON \"LiteLLM_MemoryTable\" (user_id)", (ObjectKind::Index, "LiteLLM_MemoryTable", "LiteLLM_MemoryTable_user_id_idx", "unique"), ("false", "true"))]
#[tokio::test]
async fn physical_catalog_facts(
    #[case] sql: &'static str,
    #[case] identity: (ObjectKind, &'static str, &'static str, &'static str),
    #[case] values: (&'static str, &'static str),
) {
    changed(
        sql,
        identity,
        Change::Changed {
            expected: Some(values.0.to_owned()),
            actual: Some(values.1.to_owned()),
        },
    )
    .await;
}

#[tokio::test]
async fn scalar_ids_do_not_imply_fks() {
    harness(|[reference, candidate]| async move {
        let expected = snapshot(&reference).await;
        assert!(!expected.facts.keys().any(|fact| fact.kind == ObjectKind::Constraint
            && fact.member == "invented_fk"));
        candidate.batch_execute("ALTER TABLE \"LiteLLM_ShadowEvalAttempt\" ADD CONSTRAINT invented_fk FOREIGN KEY (job_id) REFERENCES \"LiteLLM_ShadowEvalJob\" (id)").await.unwrap();
        let report = expected.compare(&snapshot(&candidate).await);
        assert!(!report.differences.is_empty());
        assert!(report.differences.iter().all(|(fact, change)| fact.member == "invented_fk"
            && matches!(change, Change::Extra { .. })));
    }).await;
}

#[tokio::test]
async fn relation_fields_are_not_columns() {
    harness(|[reference, candidate]| async move {
        let expected = snapshot(&reference).await;
        candidate
            .batch_execute("ALTER TABLE \"LiteLLM_ModelTable\" ADD COLUMN team text")
            .await
            .unwrap();
        let report = expected.compare(&snapshot(&candidate).await);
        assert_eq!(report.differences.len(), 7);
        assert!(
            report
                .differences
                .iter()
                .all(|(fact, change)| fact.kind == ObjectKind::Column
                    && fact.member == "team"
                    && matches!(change, Change::Extra { .. }))
        );
    })
    .await;
}

#[tokio::test]
async fn namespace_text_literals_survive() {
    harness(|[reference, candidate]| async move {
        let sql = "CREATE TABLE literal_test (value text DEFAULT 'public.\"JobStatus\"');";
        reference.batch_execute(sql).await.unwrap();
        candidate.batch_execute(sql).await.unwrap();
        candidate
            .batch_execute("ALTER SCHEMA public RENAME TO renamed")
            .await
            .unwrap();
        let expected = snapshot(&reference).await;
        let actual = SchemaContract::read(&**candidate, "renamed", LIMIT)
            .await
            .unwrap();
        assert!(expected.compare(&actual).differences.is_empty());
        assert_eq!(
            value(
                &actual,
                ObjectKind::Column,
                "literal_test",
                "value",
                "default"
            ),
            Some("'public.\"JobStatus\"'::text")
        );
    })
    .await;
}
