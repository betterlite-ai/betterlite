use crate::request_log_index_catalog::catalog::Relation;
use crate::request_log_indexes::partition::{PartitionTarget, build_on_partitioned_table};

use super::support::{
    API_KEY_INDEX, FakeDatabase, FixedJitter, PostgresTest, RecordingDatabase, RecordingPause,
    SPEND_LOGS, attached_children, duplicate_api_key_rows, index_builder, index_by_name,
    index_dependencies, index_oid, index_state_of, informed, install_logs, invalid_index,
    leave_invalid_index, partitioned_table, run_partition, suffix, valid_index,
};

#[tokio::test]
async fn parent_optional_empty_names_not_defaulted() {
    let fake = FakeDatabase::default().working();
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    let result = build_on_partitioned_table(
        &builder,
        index_by_name(API_KEY_INDEX),
        PartitionTarget {
            schema: builder.schema,
            table: Some(""),
            name: Some(""),
        },
    )
    .await
    .unwrap();
    assert!(!result);
    let statements = fake.state().statements.clone();
    assert!(
        statements.iter().any(|statement| statement
            .starts_with("CREATE INDEX IF NOT EXISTS \"\" ON ONLY \"public\".\"\"")),
        "{statements:?}"
    );
    assert!(
        fake.state()
            .index_states_asked
            .iter()
            .all(|name| name != API_KEY_INDEX)
    );
}

#[tokio::test]
async fn existing_invalid_parent_reused() {
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .tables
            .insert(SPEND_LOGS.to_owned(), Ok(Some(partitioned_table())));
        state.index_states.insert(
            API_KEY_INDEX.to_owned(),
            Ok(Some(invalid_index(SPEND_LOGS))),
        );
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    assert!(!run_partition(&builder, builder.schema).await);
    assert!(fake.state().statements.is_empty());
    assert!(pause.delays().is_empty());
}

#[tokio::test]
async fn first_failed_child_stops_later_children() {
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .tables
            .insert(SPEND_LOGS.to_owned(), Ok(Some(partitioned_table())));
        state
            .index_states
            .insert(API_KEY_INDEX.to_owned(), Ok(Some(valid_index(SPEND_LOGS))));
        state
            .children
            .insert(SPEND_LOGS.to_owned(), child_relations());
        state.index_states.insert(
            format!("child_a_{}", suffix()),
            Ok(Some(valid_index("somewhere_else"))),
        );
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    assert!(!run_partition(&builder, builder.schema).await);
    assert!(
        !fake
            .state()
            .index_states_asked
            .contains(&format!("child_b_{}", suffix()))
    );
}

fn child_relations() -> Vec<Relation> {
    vec![
        Relation {
            name: "child_a".to_owned(),
            schema: "public".to_owned(),
            partitioned: false,
        },
        Relation {
            name: "child_b".to_owned(),
            schema: "public".to_owned(),
            partitioned: false,
        },
    ]
}

#[tokio::test]
async fn final_parent_validity_required() {
    let fake = FakeDatabase::default().working();
    fake.configure(|state| {
        state
            .tables
            .insert(SPEND_LOGS.to_owned(), Ok(Some(partitioned_table())));
        state.index_state_sequence.push_back(Ok(None));
        state.index_state_sequence.push_back(Ok(None));
    });
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::new(&fake);
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&fake, "public", &dependencies);
    assert!(!run_partition(&builder, builder.schema).await);
    assert!(
        fake.state()
            .statements
            .iter()
            .any(|statement| statement.starts_with("CREATE INDEX IF NOT EXISTS"))
    );
    assert_eq!(fake.state().equivalents_asked, 1);
}

#[tokio::test]
async fn interrupted_build_resumes() {
    let (logs, _guard) = install_logs();
    let test = PostgresTest::new().await;
    create_plain_partitions(&test).await;
    duplicate_api_key_rows(&test.client, &format!("{SPEND_LOGS}_p1")).await;
    let child = format!("LiteLLM_SpendLogs_p1_{}", suffix());
    leave_invalid_index(
        &test.client,
        &child,
        &format!("{SPEND_LOGS}_p1"),
        "\"api_key\", \"startTime\"",
    )
    .await;
    let invalid_oid = index_oid(&test.client, &child).await.unwrap();
    let recorder = RecordingDatabase::new(test.service().await);
    let jitter = FixedJitter::new(0);
    let pause = RecordingPause::standalone();
    let dependencies = index_dependencies(&jitter, &pause);
    let builder = index_builder(&recorder, &test.schema, &dependencies);
    assert!(run_partition(&builder, builder.schema).await);
    assert_ne!(index_oid(&test.client, &child).await.unwrap(), invalid_oid);
    assert!(index_state_of(&test.client, &child).await.unwrap().1);
    assert_eq!(
        attached_children(&test.client, &test.schema, API_KEY_INDEX)
            .await
            .len(),
        2
    );
    assert!(informed(
        &logs.contents(),
        &format!(
            "Dropping the invalid index {child} left by an interrupted build on LiteLLM_SpendLogs_p1"
        )
    ));
    test.finish().await;
}

async fn create_plain_partitions(test: &PostgresTest) {
    test.client
        .batch_execute(&format!(
            "CREATE TABLE \"{SPEND_LOGS}\" (request_id text, api_key text, \"startTime\" timestamptz, litellm_call_id text) PARTITION BY RANGE (\"startTime\");
             CREATE TABLE \"{SPEND_LOGS}_p1\" PARTITION OF \"{SPEND_LOGS}\" FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');
             CREATE TABLE \"{SPEND_LOGS}_p2\" PARTITION OF \"{SPEND_LOGS}\" FOR VALUES FROM ('2026-02-01') TO ('2026-03-01');"
        ))
        .await
        .unwrap();
}
