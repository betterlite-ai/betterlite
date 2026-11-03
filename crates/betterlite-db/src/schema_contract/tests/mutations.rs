async fn changed(
    sql: &'static str,
    identity: (ObjectKind, &'static str, &'static str, &'static str),
    expected: Change,
) {
    harness(move |[reference, candidate]| async move {
        let before = snapshot(&reference).await;
        assert!(
            before
                .compare(&snapshot(&candidate).await)
                .differences
                .is_empty()
        );
        candidate.batch_execute(sql).await.unwrap();
        let after = snapshot(&candidate).await;
        let report = before.compare(&after);
        let (kind, relation, member, attribute) = identity;
        let key = Fact {
            kind,
            relation: relation.to_owned(),
            member: member.to_owned(),
            attribute: attribute.parse().unwrap(),
        };
        assert_eq!(report.differences.get(&key), Some(&expected), "{report:#?}");
        assert!(
            report
                .differences
                .keys()
                .all(|fact| fact.relation == relation)
        );
    })
    .await;
}

#[tokio::test]
async fn missing_relation() {
    changed(
        "DROP TABLE \"LiteLLM_Lens\"",
        (ObjectKind::Relation, "LiteLLM_Lens", "", "kind"),
        Change::Missing {
            expected: Some("r".to_owned()),
        },
    )
    .await;
}

#[tokio::test]
async fn quoted_column_case() {
    changed(
        "ALTER TABLE \"LiteLLM_Lens\" RENAME data TO \"Data\"",
        (ObjectKind::Column, "LiteLLM_Lens", "data", "type"),
        Change::Missing {
            expected: Some("pg_catalog.jsonb".to_owned()),
        },
    )
    .await;
}

#[tokio::test]
async fn wrong_column_order() {
    changed("ALTER TABLE \"LiteLLM_Lens\" DROP COLUMN data; ALTER TABLE \"LiteLLM_Lens\" ADD COLUMN data JSONB NOT NULL",
        (ObjectKind::Column, "LiteLLM_Lens", "data", "position"),
        Change::Changed { expected: Some("3".to_owned()), actual: Some("4".to_owned()) }).await;
}

#[rstest]
#[case::native_type(
    "ALTER TABLE \"LiteLLM_Lens\" ALTER COLUMN version TYPE BIGINT",
    "type",
    "pg_catalog.int4",
    "pg_catalog.int8"
)]
#[case::typmod(
    "ALTER TABLE \"LiteLLM_LensRun\" ALTER COLUMN created_at TYPE TIMESTAMP(6)",
    "typmod",
    "3",
    "6"
)]
#[tokio::test]
async fn wrong_type_or_typmod(
    #[case] sql: &'static str,
    #[case] attribute: &'static str,
    #[case] before: &'static str,
    #[case] after: &'static str,
) {
    let (table, column) = if attribute == "type" {
        ("LiteLLM_Lens", "version")
    } else {
        ("LiteLLM_LensRun", "created_at")
    };
    changed(
        sql,
        (ObjectKind::Column, table, column, attribute),
        Change::Changed {
            expected: Some(before.to_owned()),
            actual: Some(after.to_owned()),
        },
    )
    .await;
}

#[tokio::test]
async fn changed_nullability() {
    changed(
        "ALTER TABLE \"LiteLLM_Lens\" ALTER COLUMN data DROP NOT NULL",
        (ObjectKind::Column, "LiteLLM_Lens", "data", "not_null"),
        Change::Changed {
            expected: Some("true".to_owned()),
            actual: Some("false".to_owned()),
        },
    )
    .await;
}

#[tokio::test]
async fn missing_sql_default() {
    changed(
        "ALTER TABLE \"LiteLLM_Lens\" ALTER COLUMN version DROP DEFAULT",
        (ObjectKind::Column, "LiteLLM_Lens", "version", "default"),
        Change::Changed {
            expected: Some("0".to_owned()),
            actual: None,
        },
    )
    .await;
}

#[tokio::test]
async fn changed_default_literal() {
    changed(
        "ALTER TABLE \"LiteLLM_Lens\" ALTER COLUMN version SET DEFAULT 1",
        (ObjectKind::Column, "LiteLLM_Lens", "version", "default"),
        Change::Changed {
            expected: Some("0".to_owned()),
            actual: Some("1".to_owned()),
        },
    )
    .await;
}

#[tokio::test]
async fn primary_key_column_order() {
    changed("ALTER TABLE \"LiteLLM_TeamMembership\" DROP CONSTRAINT \"LiteLLM_TeamMembership_pkey\"; ALTER TABLE \"LiteLLM_TeamMembership\" ADD PRIMARY KEY(team_id, user_id)",
        (ObjectKind::Constraint, "LiteLLM_TeamMembership", "LiteLLM_TeamMembership_pkey", "columns"),
        Change::Changed { expected: Some("{user_id,team_id}".to_owned()), actual: Some("{team_id,user_id}".to_owned()) }).await;
}

#[tokio::test]
async fn changed_foreign_key_target() {
    changed("ALTER TABLE \"LiteLLM_TeamMembership\" DROP CONSTRAINT \"LiteLLM_TeamMembership_budget_id_fkey\"; ALTER TABLE \"LiteLLM_TeamMembership\" ADD CONSTRAINT \"LiteLLM_TeamMembership_budget_id_fkey\" FOREIGN KEY (budget_id) REFERENCES \"LiteLLM_UserTable\"(user_id) ON DELETE SET NULL ON UPDATE CASCADE",
        (ObjectKind::Constraint, "LiteLLM_TeamMembership", "LiteLLM_TeamMembership_budget_id_fkey", "target"),
        Change::Changed { expected: Some("\"\".\"LiteLLM_BudgetTable\"".to_owned()), actual: Some("\"\".\"LiteLLM_UserTable\"".to_owned()) }).await;
}

#[rstest]
#[case::delete("RESTRICT", "CASCADE", "delete_action", "n", "r")]
#[case::update("SET NULL", "RESTRICT", "update_action", "c", "r")]
#[tokio::test]
async fn changed_delete_or_update_action(
    #[case] delete: &str,
    #[case] update: &str,
    #[case] attribute: &str,
    #[case] before: &str,
    #[case] after: &str,
) {
    harness(|[reference, candidate]| async move {
        let expected = snapshot(&reference).await;
        let sql = format!("ALTER TABLE \"LiteLLM_TeamMembership\" DROP CONSTRAINT \"LiteLLM_TeamMembership_budget_id_fkey\"; ALTER TABLE \"LiteLLM_TeamMembership\" ADD CONSTRAINT \"LiteLLM_TeamMembership_budget_id_fkey\" FOREIGN KEY (budget_id) REFERENCES \"LiteLLM_BudgetTable\"(budget_id) ON DELETE {delete} ON UPDATE {update}");
        candidate.batch_execute(&sql).await.unwrap();
        let actual = snapshot(&candidate).await;
        assert_eq!(value(&expected, ObjectKind::Constraint, "LiteLLM_TeamMembership", "LiteLLM_TeamMembership_budget_id_fkey", attribute), Some(before));
        assert_eq!(value(&actual, ObjectKind::Constraint, "LiteLLM_TeamMembership", "LiteLLM_TeamMembership_budget_id_fkey", attribute), Some(after));
        assert_eq!(expected.compare(&actual).differences.len(), 1);
    }).await;
}

#[tokio::test]
async fn changed_index_direction() {
    changed("DROP INDEX \"LiteLLM_LensRun_lens_id_created_at_idx\"; CREATE INDEX \"LiteLLM_LensRun_lens_id_created_at_idx\" ON \"LiteLLM_LensRun\" (lens_id, created_at DESC)",
        (ObjectKind::Index, "LiteLLM_LensRun", "LiteLLM_LensRun_lens_id_created_at_idx", "direction"),
        Change::Changed { expected: Some("0 0".to_owned()), actual: Some("0 3".to_owned()) }).await;
}

#[tokio::test]
async fn changed_index_predicate() {
    changed("DROP INDEX \"LiteLLM_ShadowEvalJob_one_active_per_target_direction\"; CREATE UNIQUE INDEX \"LiteLLM_ShadowEvalJob_one_active_per_target_direction\" ON \"LiteLLM_ShadowEvalJob\" (target_type, target_id, direction) WHERE stopped_at IS NOT NULL",
        (ObjectKind::Index, "LiteLLM_ShadowEvalJob", "LiteLLM_ShadowEvalJob_one_active_per_target_direction", "predicate"),
        Change::Changed { expected: Some("(stopped_at IS NULL)".to_owned()), actual: Some("(stopped_at IS NOT NULL)".to_owned()) }).await;
}

#[tokio::test]
async fn mapped_aliases_column() {
    changed(
        "ALTER TABLE \"LiteLLM_ModelTable\" RENAME aliases TO model_aliases",
        (ObjectKind::Column, "LiteLLM_ModelTable", "aliases", "type"),
        Change::Missing {
            expected: Some("pg_catalog.jsonb".to_owned()),
        },
    )
    .await;
}
