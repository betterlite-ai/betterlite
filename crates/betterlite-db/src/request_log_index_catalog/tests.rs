#![cfg(test)]

use super::catalog::{
    children_without_the_index, equivalent_indexes, index_state, regclass_name, table_kind,
};
use super::definitions::{REQUEST_LOG_INDEXES, RequestLogIndex};
use betterlite_postgres_fixture::connection::PostgresFixture;
use deadpool_postgres::Client;
use rstest::rstest;

#[test]
fn registry_order_and_columns() {
    assert_eq!(REQUEST_LOG_INDEXES[0].columns(), ["api_key", "startTime"]);
    assert_eq!(REQUEST_LOG_INDEXES[1].columns(), ["litellm_call_id"]);
}

#[test]
fn child_name_matching_prefix() {
    assert_eq!(
        REQUEST_LOG_INDEXES[0].partition_index_name("part"),
        "part_api_key_startTime_idx"
    );
}

#[test]
fn child_name_nonmatching_prefix() {
    let index = RequestLogIndex {
        table: "Other",
        name: "LiteLLM_SpendLogs_idx",
        definition: "(\"x\")",
    };
    assert_eq!(
        index.partition_index_name("part"),
        "part_LiteLLM_SpendLogs_idx"
    );
}

#[test]
fn name_exactly_sixty_three_bytes() {
    let index_name = format!("T_{}", "a".repeat(61));
    let index = RequestLogIndex {
        table: "T",
        name: &index_name,
        definition: "(\"x\")",
    };
    let name = index.partition_index_name("p");
    assert_eq!(name.len(), 63);
}

#[test]
fn long_name_full_digest() {
    let index = RequestLogIndex {
        table: "T",
        name: "T_a_very_long_index_name_that_needs_truncation",
        definition: "(\"x\")",
    };
    let name = index.partition_index_name(&"p".repeat(100));
    assert_eq!(name.len(), 63);
    assert!(name.ends_with("_087344be"));
}

#[test]
fn long_unicode_name_boundary() {
    let index = RequestLogIndex {
        table: "T",
        name: "T_suffix",
        definition: "(\"x\")",
    };
    let name = index.partition_index_name(&"\u{754c}".repeat(30));
    assert!(name.is_char_boundary(54));
    assert_eq!(name.len(), 63);
}

#[rstest]
#[case::different_suffix("first", "second")]
#[case::same_prefix_different_suffix("a", "b")]
fn long_suffixes_remain_distinct(#[case] first: &str, #[case] second: &str) {
    let index = RequestLogIndex {
        table: "T",
        name: "T_suffix",
        definition: "(\"x\")",
    };
    assert_ne!(
        index.partition_index_name(&format!("{}{}", "p".repeat(100), first)),
        index.partition_index_name(&format!("{}{}", "p".repeat(100), second))
    );
}

#[test]
fn missing_relation() {
    assert_eq!(regclass_name("public", "missing"), "\"public\".\"missing\"");
}

#[test]
fn cross_schema_children_and_quoted_names() {
    assert_eq!(
        regclass_name("quoted schema", "quoted\"name"),
        "\"quoted schema\".\"quoted\"\"name\""
    );
}

#[tokio::test]
async fn postgres_equivalent_default_btree() {
    let (fixture, client, schema) = database().await;
    client
        .batch_execute(
            "CREATE TABLE parent (api_key TEXT, \"startTime\" TEXT); CREATE INDEX expected_idx ON parent (api_key, \"startTime\"); CREATE INDEX equivalent_idx ON parent (api_key, \"startTime\"); CREATE UNIQUE INDEX unique_idx ON parent (api_key, \"startTime\"); CREATE INDEX reversed_idx ON parent (\"startTime\", api_key); CREATE INDEX predicate_idx ON parent (api_key, \"startTime\") WHERE api_key IS NOT NULL; CREATE INDEX included_idx ON parent (api_key) INCLUDE (\"startTime\"); CREATE INDEX descending_idx ON parent (api_key DESC, \"startTime\" NULLS FIRST);",
        )
        .await
        .unwrap();
    let matches = equivalent_indexes(
        &client,
        &schema,
        "parent",
        "expected_idx",
        &RequestLogIndex {
            table: "parent",
            name: "expected_idx",
            definition: "(\"api_key\", \"startTime\")",
        },
    )
    .await
    .unwrap();
    assert_eq!(
        matches
            .iter()
            .map(|index| index.name.as_str())
            .collect::<Vec<_>>(),
        ["equivalent_idx"]
    );
    finish(fixture).await;
}

#[tokio::test]
async fn postgres_inspection_never_mutates() {
    let (fixture, client, schema) = database().await;
    let quoted_schema = format!("q {schema}");
    create_partition_schema(&client, &quoted_schema).await;
    let before = index_oid(&client, &quoted_schema).await;
    assert!(
        table_kind(&client, &quoted_schema, "parent")
            .await
            .unwrap()
            .unwrap()
            .partitioned
    );
    assert_eq!(
        index_state(&client, &quoted_schema, "parent_idx")
            .await
            .unwrap()
            .unwrap()
            .table,
        "parent"
    );
    let children = children_without_the_index(&client, &quoted_schema, "parent", "parent_idx")
        .await
        .unwrap();
    assert!(children.is_empty());
    let after = index_oid(&client, &quoted_schema).await;
    assert_eq!(before, after);
    finish(fixture).await;
}

async fn database() -> (PostgresFixture, Client, String) {
    use clap::Parser;
    use deadpool_postgres::{Manager, Pool};
    use secrecy::{ExposeSecret as _, SecretString};
    use tokio_postgres::NoTls;

    #[derive(Parser)]
    struct Environment {
        #[arg(long, env = "POSTGRES_TEST_URL")]
        url: Option<String>,
    }
    let url = Environment::parse_from(["request-log-index-catalog-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(1)
        .build()
        .unwrap();
    let client = pool.get().await.unwrap();
    let schema = client
        .query_one("SELECT current_schema()", &[])
        .await
        .unwrap()
        .get(0);
    (fixture, client, schema)
}

async fn finish(fixture: PostgresFixture) {
    fixture.close_async().await.unwrap();
}

async fn create_partition_schema(client: &Client, schema: &str) {
    let quoted_schema = schema.replace('"', "\"\"");
    client
        .batch_execute(&format!(
            "CREATE SCHEMA \"{quoted_schema}\"; CREATE TABLE \"{quoted_schema}\".parent (id INT, api_key TEXT) PARTITION BY RANGE (id); CREATE TABLE child_a PARTITION OF \"{quoted_schema}\".parent FOR VALUES FROM (0) TO (10); CREATE TABLE \"{quoted_schema}\".child_b PARTITION OF \"{quoted_schema}\".parent FOR VALUES FROM (10) TO (20); CREATE INDEX parent_idx ON \"{quoted_schema}\".parent (api_key);"
        ))
        .await
        .unwrap();
}

async fn index_oid(client: &Client, schema: &str) -> u32 {
    client
        .query_one(
            "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = 'parent_idx'",
            &[&schema],
        )
        .await
        .unwrap()
        .get(0)
}
