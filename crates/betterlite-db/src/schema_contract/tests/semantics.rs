use crate::request_log_index_catalog::definitions::REQUEST_LOG_INDEXES;
use std::collections::BTreeSet;
#[tokio::test]
async fn ninety_model_inventory() {
    let models = include_str!("../fixtures/root/schema.prisma")
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("model ")?;
            rest.split_whitespace().next()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(models.len(), 90);
    harness(|[reference, _candidate]| async move {
        let catalog = snapshot(&reference).await;
        let tables = catalog
            .facts
            .keys()
            .filter(|fact| fact.kind == ObjectKind::Relation)
            .map(|fact| fact.relation.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(models, tables);
    })
    .await;
}

#[tokio::test]
async fn sql_only_partial_indexes() {
    harness(|[reference, _candidate]| async move {
        let catalog = snapshot(&reference).await;
        assert_eq!(
            value(
                &catalog,
                ObjectKind::Index,
                "LiteLLM_ShadowEvalJob",
                "LiteLLM_ShadowEvalJob_one_active_per_target_direction",
                "predicate"
            ),
            Some("(stopped_at IS NULL)")
        );
        assert_eq!(
            value(
                &catalog,
                ObjectKind::Index,
                "LiteLLM_ShadowEvalJob",
                "LiteLLM_ShadowEvalJob_one_active_per_target_direction",
                "unique"
            ),
            Some("true")
        );
        assert_eq!(
            value(
                &catalog,
                ObjectKind::Index,
                "LiteLLM_AutoRouterBaselineComparison",
                "idx_autorouter_baseline_dirty",
                "predicate"
            ),
            Some("((NOT retired) AND (revision <> published_revision))")
        );
    })
    .await;
}

#[tokio::test]
async fn optional_managed_indexes_absent() {
    harness(|[reference, candidate]| async move {
        let expected = snapshot(&reference).await;
        let report = expected.compare(&snapshot(&candidate).await);
        assert!(report.optional_expected.is_empty());
        assert!(report.optional_actual.is_empty());
        assert!(report.differences.is_empty());
    })
    .await;
}

#[rstest]
#[case::api_key(0)]
#[case::call_id(1)]
#[tokio::test]
async fn optional_managed_indexes_present(#[case] index_position: usize) {
    harness(|[reference, candidate]| async move {
        let index = REQUEST_LOG_INDEXES[index_position];
        candidate
            .batch_execute(&format!(
                "CREATE INDEX \"{}\" ON \"{}\" {}",
                index.name, index.table, index.definition
            ))
            .await
            .unwrap();
        let expected = snapshot(&reference).await;
        let actual = snapshot(&candidate).await;
        let report = expected.compare(&actual);
        assert!(report.differences.is_empty());
        assert_eq!(report.optional_actual.len(), 8);
        assert!(report.optional_expected.is_empty());
        let reverse = actual.compare(&expected);
        assert!(reverse.differences.is_empty());
        assert_eq!(reverse.optional_expected, report.optional_actual);
    })
    .await;
}

#[rstest]
#[case::uuid("LiteLLM_MemoryTable", "memory_id")]
#[case::cuid("LiteLLM_CronJob", "cronjob_id")]
#[case::updated_at("LiteLLM_BudgetWindowSpend", "updated_at")]
#[tokio::test]
async fn client_defaults_are_not_sql_defaults(
    #[case] table: &'static str,
    #[case] column: &'static str,
) {
    harness(|[reference, candidate]| async move {
        let expected = snapshot(&reference).await;
        assert_eq!(
            value(&expected, ObjectKind::Column, table, column, "default"),
            None
        );
        let sql = if column == "updated_at" {
            format!("ALTER TABLE \"{table}\" ALTER COLUMN \"{column}\" SET DEFAULT now()")
        } else {
            format!("ALTER TABLE \"{table}\" ALTER COLUMN \"{column}\" SET DEFAULT 'invented'")
        };
        candidate.batch_execute(&sql).await.unwrap();
        assert_eq!(
            expected
                .compare(&snapshot(&candidate).await)
                .differences
                .len(),
            1
        );
    })
    .await;
}

#[rstest]
#[case::sql_null("NULL", None)]
#[case::json_null("'null'::jsonb", Some("'null'::jsonb"))]
#[case::json_object("'{}'::jsonb", Some("'{}'::jsonb"))]
#[case::json_array("'[]'::jsonb", Some("'[]'::jsonb"))]
#[case::text("'{}'::text", Some("'{}'::text"))]
#[case::array("'{}'::text[]", Some("'{}'::text[]"))]
#[tokio::test]
async fn null_json_text_and_array_defaults(
    #[case] expression: &str,
    #[case] expected_default: Option<&str>,
) {
    harness(|[reference, candidate]| async move {
        let typ = if expression.ends_with("text[]") {
            "text[]"
        } else if expression.ends_with("text") {
            "text"
        } else {
            "jsonb"
        };
        let base = format!("CREATE TABLE defaults (value {typ})");
        reference.batch_execute(&base).await.unwrap();
        candidate.batch_execute(&base).await.unwrap();
        candidate
            .batch_execute(&format!(
                "ALTER TABLE defaults ALTER COLUMN value SET DEFAULT {expression}"
            ))
            .await
            .unwrap();
        let before = snapshot(&reference).await;
        let after = snapshot(&candidate).await;
        assert_eq!(
            value(&after, ObjectKind::Column, "defaults", "value", "default"),
            expected_default
        );
        assert_eq!(
            before.compare(&after).differences.len(),
            usize::from(expected_default.is_some())
        );
    })
    .await;
}

#[tokio::test]
async fn nullable_unique_components() {
    harness(|[reference, candidate]| async move {
        let index = "LiteLLM_AgentIdentity_issuer_service_principal_id_key";
        let expected = snapshot(&reference).await;
        assert_eq!(value(&expected, ObjectKind::Column, "LiteLLM_AgentIdentity", "service_principal_id", "not_null"), Some("false"));
        assert_eq!(value(&expected, ObjectKind::Index, "LiteLLM_AgentIdentity", index, "nulls_not_distinct"), Some("false"));
        candidate.batch_execute(&format!("DROP INDEX \"{index}\"; CREATE UNIQUE INDEX \"{index}\" ON \"LiteLLM_AgentIdentity\" (issuer, service_principal_id) NULLS NOT DISTINCT")).await.unwrap();
        let report = expected.compare(&snapshot(&candidate).await);
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.differences.keys().next().unwrap().attribute, Attribute::NullsNotDistinct);
    }).await;
}

#[tokio::test]
async fn global_memory_key() {
    changed("DROP INDEX \"LiteLLM_MemoryTable_key_key\"; CREATE UNIQUE INDEX \"LiteLLM_MemoryTable_key_key\" ON \"LiteLLM_MemoryTable\" (key, user_id)",
        (ObjectKind::Index, "LiteLLM_MemoryTable", "LiteLLM_MemoryTable_key_key", "keys"),
        Change::Changed { expected: Some("{key}".to_owned()), actual: Some("{key,user_id}".to_owned()) }).await;
}

#[tokio::test]
async fn invalid_index() {
    harness(|[reference, candidate]| async move {
        reference.batch_execute("CREATE TABLE invalid_test (id int); CREATE UNIQUE INDEX invalid_idx ON invalid_test(id)").await.unwrap();
        candidate.batch_execute("CREATE TABLE invalid_test (id int); INSERT INTO invalid_test VALUES (1), (1)").await.unwrap();
        assert!(candidate.batch_execute("CREATE UNIQUE INDEX CONCURRENTLY invalid_idx ON invalid_test(id)").await.is_err());
        let expected = snapshot(&reference).await;
        let actual = snapshot(&candidate).await;
        assert_eq!(value(&actual, ObjectKind::Index, "invalid_test", "invalid_idx", "valid"), Some("false"));
        let report = expected.compare(&actual);
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.differences.keys().next().unwrap().attribute, Attribute::Valid);
    }).await;
}

#[rstest]
#[case::sequence(
    "ALTER SEQUENCE \"LiteLLM_ModelTable_id_seq\" INCREMENT BY 2",
    ObjectKind::Sequence,
    "LiteLLM_ModelTable_id_seq",
    "increment",
    ("1", "2")
)]
#[case::labels(
    "ALTER TYPE \"JobStatus\" RENAME VALUE 'ACTIVE' TO 'LIVE'",
    ObjectKind::Enum,
    "JobStatus",
    "labels",
    ("{ACTIVE,INACTIVE}", "{LIVE,INACTIVE}")
)]
#[tokio::test]
async fn serial_sequence_and_enum_labels(
    #[case] sql: &'static str,
    #[case] kind: ObjectKind,
    #[case] relation: &'static str,
    #[case] attribute: &'static str,
    #[case] values: (&'static str, &'static str),
) {
    changed(
        sql,
        (kind, relation, "", attribute),
        Change::Changed {
            expected: Some(values.0.to_owned()),
            actual: Some(values.1.to_owned()),
        },
    )
    .await;
}
