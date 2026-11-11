#![cfg(test)]

use std::env::split_paths;
use std::ffi::OsString;

use clap::Parser;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};

use crate::connection::{PostgresFixture, RandomDatabaseName};
use crate::daily_usage::{self, DailyUsageError, DailyUsageRow, DailyUsageTable};
use crate::error::FixtureError;
use crate::seed::{FixtureSeed, KeyRow, TeamRow, UserRow};
use crate::tests::postgres_server;

type ColumnExpectation = (&'static str, &'static str, bool, &'static str);
type IndexExpectation = (&'static str, &'static [&'static str], bool);

const USER_COLUMNS: &[ColumnExpectation] = &[
    ("id", "text", false, ""),
    ("user_id", "text", true, ""),
    ("date", "text", false, ""),
    ("api_key", "text", false, ""),
    ("model", "text", true, ""),
    ("model_group", "text", true, ""),
    ("custom_llm_provider", "text", true, ""),
    ("prompt_tokens", "bigint", false, "0"),
    ("completion_tokens", "bigint", false, "0"),
    ("spend", "double precision", false, "0.0"),
    (
        "created_at",
        "timestamp without time zone",
        false,
        "CURRENT_TIMESTAMP",
    ),
    ("updated_at", "timestamp without time zone", false, ""),
    ("api_requests", "bigint", false, "0"),
    ("failed_requests", "bigint", false, "0"),
    ("successful_requests", "bigint", false, "0"),
    ("cache_creation_input_tokens", "bigint", false, "0"),
    ("cache_read_input_tokens", "bigint", false, "0"),
    ("mcp_namespaced_tool_name", "text", true, ""),
    ("endpoint", "text", true, ""),
    ("compression_saved_tokens", "bigint", false, "0"),
    (
        "compression_savings_spend",
        "double precision",
        false,
        "0.0",
    ),
    (
        "prompt_caching_savings_spend",
        "double precision",
        false,
        "0.0",
    ),
    (
        "gateway_injected_caching_savings_spend",
        "double precision",
        false,
        "0.0",
    ),
    ("autorouter_savings_spend", "double precision", false, "0.0"),
    ("total_response_time_ms", "bigint", false, "0"),
    ("timed_requests", "bigint", false, "0"),
];

const TEAM_COLUMNS: &[ColumnExpectation] = &[
    ("id", "text", false, ""),
    ("team_id", "text", true, ""),
    ("date", "text", false, ""),
    ("api_key", "text", false, ""),
    ("model", "text", true, ""),
    ("model_group", "text", true, ""),
    ("custom_llm_provider", "text", true, ""),
    ("prompt_tokens", "bigint", false, "0"),
    ("completion_tokens", "bigint", false, "0"),
    ("spend", "double precision", false, "0.0"),
    ("api_requests", "bigint", false, "0"),
    ("successful_requests", "bigint", false, "0"),
    ("failed_requests", "bigint", false, "0"),
    (
        "created_at",
        "timestamp without time zone",
        false,
        "CURRENT_TIMESTAMP",
    ),
    ("updated_at", "timestamp without time zone", false, ""),
    ("cache_creation_input_tokens", "bigint", false, "0"),
    ("cache_read_input_tokens", "bigint", false, "0"),
    ("mcp_namespaced_tool_name", "text", true, ""),
    ("endpoint", "text", true, ""),
    ("ptu_flat_cost", "double precision", false, "0.0"),
    ("compression_saved_tokens", "bigint", false, "0"),
    (
        "compression_savings_spend",
        "double precision",
        false,
        "0.0",
    ),
    (
        "prompt_caching_savings_spend",
        "double precision",
        false,
        "0.0",
    ),
    ("autorouter_savings_spend", "double precision", false, "0.0"),
    (
        "gateway_injected_caching_savings_spend",
        "double precision",
        false,
        "0.0",
    ),
    ("total_response_time_ms", "bigint", false, "0"),
    ("timed_requests", "bigint", false, "0"),
];

const TAG_COLUMNS: &[ColumnExpectation] = &[
    ("id", "text", false, ""),
    ("tag", "text", true, ""),
    ("date", "text", false, ""),
    ("api_key", "text", false, ""),
    ("model", "text", true, ""),
    ("model_group", "text", true, ""),
    ("custom_llm_provider", "text", true, ""),
    ("prompt_tokens", "bigint", false, "0"),
    ("completion_tokens", "bigint", false, "0"),
    ("cache_read_input_tokens", "bigint", false, "0"),
    ("cache_creation_input_tokens", "bigint", false, "0"),
    ("spend", "double precision", false, "0.0"),
    ("api_requests", "bigint", false, "0"),
    ("successful_requests", "bigint", false, "0"),
    ("failed_requests", "bigint", false, "0"),
    (
        "created_at",
        "timestamp without time zone",
        false,
        "CURRENT_TIMESTAMP",
    ),
    ("updated_at", "timestamp without time zone", false, ""),
    ("mcp_namespaced_tool_name", "text", true, ""),
    ("request_id", "text", true, ""),
    ("endpoint", "text", true, ""),
    ("compression_saved_tokens", "bigint", false, "0"),
    (
        "compression_savings_spend",
        "double precision",
        false,
        "0.0",
    ),
    (
        "prompt_caching_savings_spend",
        "double precision",
        false,
        "0.0",
    ),
    ("autorouter_savings_spend", "double precision", false, "0.0"),
    (
        "gateway_injected_caching_savings_spend",
        "double precision",
        false,
        "0.0",
    ),
    ("total_response_time_ms", "bigint", false, "0"),
    ("timed_requests", "bigint", false, "0"),
];

const USER_INDEXES: &[IndexExpectation] = &[
    ("LiteLLM_DailyUserSpend_pkey", &["id"], true),
    ("LiteLLM_DailyUserSpend_date_idx", &["date"], false),
    (
        "LiteLLM_DailyUserSpend_user_id_date_idx",
        &["user_id", "date"],
        false,
    ),
    ("LiteLLM_DailyUserSpend_api_key_idx", &["api_key"], false),
    ("LiteLLM_DailyUserSpend_model_idx", &["model"], false),
    (
        "LiteLLM_DailyUserSpend_mcp_namespaced_tool_name_idx",
        &["mcp_namespaced_tool_name"],
        false,
    ),
    ("LiteLLM_DailyUserSpend_endpoint_idx", &["endpoint"], false),
    (
        "LiteLLM_DailyUserSpend_user_id_date_api_key_model_custom_ll_key",
        &[
            "user_id",
            "date",
            "api_key",
            "model",
            "custom_llm_provider",
            "mcp_namespaced_tool_name",
            "endpoint",
        ],
        true,
    ),
];

const TEAM_INDEXES: &[IndexExpectation] = &[
    ("LiteLLM_DailyTeamSpend_pkey", &["id"], true),
    ("LiteLLM_DailyTeamSpend_date_idx", &["date"], false),
    (
        "LiteLLM_DailyTeamSpend_team_id_date_idx",
        &["team_id", "date"],
        false,
    ),
    ("LiteLLM_DailyTeamSpend_api_key_idx", &["api_key"], false),
    ("LiteLLM_DailyTeamSpend_model_idx", &["model"], false),
    (
        "LiteLLM_DailyTeamSpend_mcp_namespaced_tool_name_idx",
        &["mcp_namespaced_tool_name"],
        false,
    ),
    ("LiteLLM_DailyTeamSpend_endpoint_idx", &["endpoint"], false),
    (
        "LiteLLM_DailyTeamSpend_team_id_date_api_key_model_custom_ll_key",
        &[
            "team_id",
            "date",
            "api_key",
            "model",
            "custom_llm_provider",
            "mcp_namespaced_tool_name",
            "endpoint",
        ],
        true,
    ),
];

const TAG_INDEXES: &[IndexExpectation] = &[
    ("LiteLLM_DailyTagSpend_pkey", &["id"], true),
    ("LiteLLM_DailyTagSpend_date_idx", &["date"], false),
    (
        "LiteLLM_DailyTagSpend_tag_date_idx",
        &["tag", "date"],
        false,
    ),
    ("LiteLLM_DailyTagSpend_api_key_idx", &["api_key"], false),
    ("LiteLLM_DailyTagSpend_model_idx", &["model"], false),
    (
        "LiteLLM_DailyTagSpend_mcp_namespaced_tool_name_idx",
        &["mcp_namespaced_tool_name"],
        false,
    ),
    ("LiteLLM_DailyTagSpend_endpoint_idx", &["endpoint"], false),
    (
        "LiteLLM_DailyTagSpend_tag_date_api_key_model_custom_llm_pro_key",
        &[
            "tag",
            "date",
            "api_key",
            "model",
            "custom_llm_provider",
            "mcp_namespaced_tool_name",
            "endpoint",
        ],
        true,
    ),
];

#[derive(Parser)]
struct TestEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    postgres_test_url: Option<String>,
    #[arg(long, env = "PATH")]
    path: Option<OsString>,
}

fn postgres_available() -> bool {
    let environment = TestEnvironment::parse_from(["daily-usage-fixture-tests"]);
    environment.postgres_test_url.is_some()
        || environment.path.is_some_and(|path| {
            split_paths(&path).any(|directory| directory.join("docker").is_file())
        })
}

async fn provision(admin_url: &SecretString, seed: &FixtureSeed) -> PostgresFixture {
    PostgresFixture::provision_with_name_generator(Some(admin_url), seed, &mut RandomDatabaseName)
        .resolve()
        .await
        .unwrap()
}

async fn connect(url: &SecretString) -> (Client, JoinSet<()>) {
    let (client, connection) = tokio_postgres::connect(url.expose_secret(), NoTls)
        .await
        .unwrap();
    let mut driver = JoinSet::new();
    driver.spawn(async move {
        connection.await.unwrap();
    });
    (client, driver)
}

async fn disconnect(client: Client, mut driver: JoinSet<()>) {
    drop(client);
    driver.join_next().await.unwrap().unwrap();
}

fn usage_seed_with(rows: Vec<DailyUsageRow>) -> FixtureSeed {
    FixtureSeed {
        users: vec![
            UserRow {
                user_id: "alice".to_owned(),
                user_email: None,
            },
            UserRow {
                user_id: "bob".to_owned(),
                user_email: None,
            },
        ],
        keys: vec![
            KeyRow {
                token: SecretString::from("sk-alice-token"),
                user_id: "alice".to_owned(),
                team_id: None,
                role: None,
            },
            KeyRow {
                token: SecretString::from("sk-bob-token"),
                user_id: "bob".to_owned(),
                team_id: None,
                role: None,
            },
        ],
        teams: vec![TeamRow {
            team_id: "team-red".to_owned(),
            team_alias: None,
        }],
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
        daily_usage: Some(rows),
        finops: None,
        discovery_skills: None,
        search_tools: None,
        dashboard: None,
        dashboard_governance: None,
    }
}

fn valid_row() -> DailyUsageRow {
    DailyUsageRow {
        table: DailyUsageTable::User,
        id: "row".to_owned(),
        entity_id: Some("alice".to_owned()),
        date: "2025-01-01".to_owned(),
        key_index: 0,
        model: None,
        custom_llm_provider: None,
        model_group: None,
        spend: Decimal::ZERO,
        prompt_tokens: 0,
        completion_tokens: 0,
        api_requests: 0,
        successful_requests: 0,
        failed_requests: 0,
    }
}

fn usage_rows() -> Vec<DailyUsageRow> {
    let mut user = valid_row();
    user.id = "shared-row".to_owned();
    user.model = Some("openai/gpt-5".to_owned());
    user.custom_llm_provider = Some("openai".to_owned());
    user.model_group = Some("default".to_owned());
    user.spend = Decimal::new(125, 2);
    user.prompt_tokens = 100;
    user.completion_tokens = 50;
    user.api_requests = 2;
    user.successful_requests = 1;
    user.failed_requests = 1;

    let mut team = valid_row();
    team.table = DailyUsageTable::Team;
    team.id = "shared-row".to_owned();
    team.entity_id = Some("team-red".to_owned());
    team.date = "2025-02-02".to_owned();
    team.key_index = 1;
    team.spend = Decimal::new(-50, 2);

    let mut tag = valid_row();
    tag.table = DailyUsageTable::Tag;
    tag.id = "shared-row".to_owned();
    tag.entity_id = Some("tag-blue".to_owned());
    tag.date = "2025-03-03".to_owned();
    tag.model = Some("openai/gpt-5".to_owned());
    tag.custom_llm_provider = Some("openai".to_owned());
    tag.prompt_tokens = 7;
    tag.completion_tokens = 3;
    tag.api_requests = 1;
    tag.successful_requests = 1;
    vec![user, team, tag]
}

fn usage_seed() -> FixtureSeed {
    usage_seed_with(usage_rows())
}

async fn assert_table_schema(client: &Client, table: &str, expected: &[ColumnExpectation]) {
    let statement = "SELECT column_name::text, data_type::text, is_nullable::text, \
        coalesce(column_default, '')::text, coalesce(datetime_precision::text, '')::text \
        FROM information_schema.columns \
        WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position";
    let rows = client.query(statement, &[&table]).await.unwrap();
    assert_eq!(rows.len(), expected.len(), "{table} column count");
    for (row, (name, data_type, nullable, default)) in rows.iter().zip(expected) {
        assert_eq!(
            row.get::<_, String>(0),
            *name,
            "{table} column order at {name}"
        );
        assert_eq!(row.get::<_, String>(1), *data_type, "{table}.{name} type");
        assert_eq!(
            row.get::<_, String>(2) == "YES",
            *nullable,
            "{table}.{name} nullability"
        );
        assert_eq!(row.get::<_, String>(3), *default, "{table}.{name} default");
        let precision = if *data_type == "timestamp without time zone" {
            "3"
        } else {
            ""
        };
        assert_eq!(
            row.get::<_, String>(4),
            precision,
            "{table}.{name} datetime precision"
        );
    }
}

async fn assert_index(client: &Client, table: &str, name: &str, columns: &[&str], unique: bool) {
    let statement = "SELECT i.indisunique, array_agg(a.attname::text ORDER BY k.ord) \
        FROM pg_index i \
        JOIN pg_class t ON t.oid = i.indrelid \
        JOIN pg_class ic ON ic.oid = i.indexrelid \
        JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true \
        JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum \
        WHERE t.relname = $1 AND ic.relname = $2 GROUP BY i.indisunique";
    let row = client.query_one(statement, &[&table, &name]).await.unwrap();
    assert_eq!(
        row.get::<_, Vec<String>>(1),
        columns,
        "{table} {name} columns"
    );
    assert_eq!(row.get::<_, bool>(0), unique, "{table} {name} uniqueness");
}

async fn assert_table_indexes(client: &Client, table: &str, expected: &[IndexExpectation]) {
    let count_statement = "SELECT count(*) FROM pg_indexes \
        WHERE schemaname = current_schema() AND tablename = $1";
    let count: i64 = client
        .query_one(count_statement, &[&table])
        .await
        .unwrap()
        .get(0);
    assert_eq!(usize::try_from(count).unwrap(), expected.len());
    for (name, columns, unique) in expected {
        assert_index(client, table, name, columns, *unique).await;
    }
}

enum TableState {
    Absent,
    Empty,
}

async fn assert_table_state(client: &Client, table: &str, state: TableState) {
    let statement = "SELECT count(*) FROM information_schema.tables \
        WHERE table_schema = current_schema() AND table_name = $1";
    let tables: i64 = client.query_one(statement, &[&table]).await.unwrap().get(0);
    match state {
        TableState::Absent => assert_eq!(
            tables, 0,
            "{table} must stay absent without a daily_usage seed"
        ),
        TableState::Empty => {
            let rows: i64 = client
                .query_one(&format!("SELECT count(*) FROM \"{table}\""), &[])
                .await
                .unwrap()
                .get(0);
            assert_eq!((tables, rows), (1, 0), "{table} must install empty");
        }
    }
}

#[tokio::test]
async fn usage_schema_matches_pin() {
    if !postgres_available() {
        return;
    }
    let (_server, admin_url) = postgres_server().await;
    let fixture = provision(&admin_url, &usage_seed_with(Vec::new())).await;
    let (client, driver) = connect(&fixture.options.url).await;
    assert_table_schema(&client, "LiteLLM_DailyUserSpend", USER_COLUMNS).await;
    assert_table_schema(&client, "LiteLLM_DailyTeamSpend", TEAM_COLUMNS).await;
    assert_table_schema(&client, "LiteLLM_DailyTagSpend", TAG_COLUMNS).await;
    assert_table_indexes(&client, "LiteLLM_DailyUserSpend", USER_INDEXES).await;
    assert_table_indexes(&client, "LiteLLM_DailyTeamSpend", TEAM_INDEXES).await;
    assert_table_indexes(&client, "LiteLLM_DailyTagSpend", TAG_INDEXES).await;
    assert_table_state(&client, "LiteLLM_DailyUserSpend", TableState::Empty).await;
    assert_table_state(&client, "LiteLLM_DailyTeamSpend", TableState::Empty).await;
    assert_table_state(&client, "LiteLLM_DailyTagSpend", TableState::Empty).await;
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();

    let baseline = provision(&admin_url, &FixtureSeed::default()).await;
    let (baseline_client, baseline_driver) = connect(&baseline.options.url).await;
    assert_table_state(
        &baseline_client,
        "LiteLLM_DailyUserSpend",
        TableState::Absent,
    )
    .await;
    assert_table_state(
        &baseline_client,
        "LiteLLM_DailyTeamSpend",
        TableState::Absent,
    )
    .await;
    assert_table_state(
        &baseline_client,
        "LiteLLM_DailyTagSpend",
        TableState::Absent,
    )
    .await;
    disconnect(baseline_client, baseline_driver).await;
    baseline.close_async().await.unwrap();
}

async fn stored_row(client: &Client, statement: &str) -> String {
    client.query_one(statement, &[]).await.unwrap().get(0)
}

const USER_ROW_SQL: &str = "SELECT concat_ws('|', \"id\", coalesce(\"user_id\", '<null>'), \"date\", \
    \"api_key\", coalesce(\"model\", '<null>'), coalesce(\"model_group\", '<null>'), \
    coalesce(\"custom_llm_provider\", '<null>'), \"spend\"::text, \"prompt_tokens\"::text, \
    \"completion_tokens\"::text, \"api_requests\"::text, \"successful_requests\"::text, \
    \"failed_requests\"::text, coalesce(\"mcp_namespaced_tool_name\", '<null>'), \
    coalesce(\"endpoint\", '<null>'), \
    (\"created_at\" = '2026-01-01T00:00:00'::timestamp(3))::text, \
    (\"updated_at\" = '2026-01-01T00:00:00'::timestamp(3))::text) \
    FROM \"LiteLLM_DailyUserSpend\"";

const TEAM_ROW_SQL: &str = "SELECT concat_ws('|', \"id\", coalesce(\"team_id\", '<null>'), \"date\", \
    \"api_key\", coalesce(\"model\", '<null>'), coalesce(\"model_group\", '<null>'), \
    coalesce(\"custom_llm_provider\", '<null>'), \"spend\"::text, \"prompt_tokens\"::text, \
    \"completion_tokens\"::text, \"api_requests\"::text, \"successful_requests\"::text, \
    \"failed_requests\"::text, \"ptu_flat_cost\"::text, \
    (\"created_at\" = '2026-01-01T00:00:00'::timestamp(3))::text) \
    FROM \"LiteLLM_DailyTeamSpend\"";

const TAG_ROW_SQL: &str = "SELECT concat_ws('|', \"id\", coalesce(\"tag\", '<null>'), \
    coalesce(\"request_id\", '<null>'), \"date\", \"api_key\", \"spend\"::text, \
    \"prompt_tokens\"::text, \"completion_tokens\"::text, \"api_requests\"::text, \
    \"successful_requests\"::text, \"failed_requests\"::text, \
    (\"updated_at\" = '2026-01-01T00:00:00'::timestamp(3))::text) \
    FROM \"LiteLLM_DailyTagSpend\"";

fn expected_user_row() -> String {
    format!(
        "shared-row|alice|2025-01-01|{}|openai/gpt-5|default|openai|1.25|100|50|2|1|1|<null>|<null>|true|true",
        FixtureSeed::key_hash("sk-alice-token")
    )
}

fn expected_team_row() -> String {
    format!(
        "shared-row|team-red|2025-02-02|{}|<null>|<null>|<null>|-0.5|0|0|0|0|0|0|true",
        FixtureSeed::key_hash("sk-bob-token")
    )
}

fn expected_tag_row() -> String {
    format!(
        "shared-row|tag-blue|<null>|2025-03-03|{}|0|7|3|1|1|0|true",
        FixtureSeed::key_hash("sk-alice-token")
    )
}

#[tokio::test]
async fn usage_seeds_three_tables() {
    if !postgres_available() {
        return;
    }
    let (_server, admin_url) = postgres_server().await;
    let fixture = provision(&admin_url, &usage_seed()).await;
    let (client, driver) = connect(&fixture.options.url).await;
    let user = stored_row(&client, USER_ROW_SQL).await;
    assert_eq!(user, expected_user_row());
    assert!(
        !user.contains("sk-alice-token"),
        "the raw key must not be stored"
    );
    let team = stored_row(&client, TEAM_ROW_SQL).await;
    assert_eq!(team, expected_team_row());
    let tag = stored_row(&client, TAG_ROW_SQL).await;
    assert_eq!(tag, expected_tag_row());
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn usage_databases_are_independent() {
    if !postgres_available() {
        return;
    }
    let seed = usage_seed();
    let (_server, admin_url) = postgres_server().await;
    let first = provision(&admin_url, &seed).await;
    let second = provision(&admin_url, &seed).await;
    let (writer, driver) = connect(&first.options.url).await;
    writer
        .execute(
            "UPDATE \"LiteLLM_DailyUserSpend\" SET \"spend\" = 99.5 WHERE \"id\" = 'shared-row'",
            &[],
        )
        .await
        .unwrap();
    disconnect(writer, driver).await;
    assert_eq!(spend_of(&first, "LiteLLM_DailyUserSpend").await, "99.5");
    assert_eq!(spend_of(&second, "LiteLLM_DailyUserSpend").await, "1.25");
    assert_eq!(spend_of(&second, "LiteLLM_DailyTeamSpend").await, "-0.5");
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
}

async fn spend_of(fixture: &PostgresFixture, table: &str) -> String {
    let (client, driver) = connect(&fixture.options.url).await;
    let statement = format!("SELECT \"spend\"::text FROM \"{table}\" WHERE \"id\" = 'shared-row'");
    let spend: String = client.query_one(&statement, &[]).await.unwrap().get(0);
    disconnect(client, driver).await;
    spend
}

#[derive(Debug)]
enum ExpectedError {
    TooManyRows,
    UnknownKey,
    DuplicateRow,
    TextTooLong(&'static str),
    NegativeCounter(&'static str),
}

fn assert_usage_error(error: FixtureError, expected: &ExpectedError) {
    let FixtureError::DailyUsage(source) = error else {
        panic!("expected a daily usage error, got {error:?}");
    };
    let matched = match &source {
        DailyUsageError::TooManyRows { .. } => matches!(expected, ExpectedError::TooManyRows),
        DailyUsageError::UnknownKey { .. } => matches!(expected, ExpectedError::UnknownKey),
        DailyUsageError::DuplicateRow { .. } => matches!(expected, ExpectedError::DuplicateRow),
        DailyUsageError::TextTooLong { field, .. } => {
            matches!(expected, ExpectedError::TextTooLong(wanted) if field == wanted)
        }
        DailyUsageError::NegativeCounter { field, .. } => {
            matches!(expected, ExpectedError::NegativeCounter(wanted) if field == wanted)
        }
        DailyUsageError::Database(_) => false,
    };
    assert!(matched, "unexpected daily usage error {source:?}");
}

fn unique_rows(count: usize) -> Vec<DailyUsageRow> {
    (0..count)
        .map(|index| {
            let mut row = valid_row();
            row.id = format!("row-{index}");
            row
        })
        .collect()
}

fn oversized_rows() -> Vec<DailyUsageRow> {
    unique_rows(257)
}

fn boundary_rows() -> Vec<DailyUsageRow> {
    let mut rows = unique_rows(256);
    let first = rows.first_mut().expect("boundary rows are not empty");
    first.id = "i".repeat(64);
    first.date = "d".repeat(64);
    first.entity_id = Some("e".repeat(4096));
    first.model = Some("m".repeat(4096));
    first.model_group = Some("g".repeat(4096));
    first.custom_llm_provider = Some("p".repeat(4096));
    rows
}

fn rows_with_id(bytes: usize) -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.id = "x".repeat(bytes);
    vec![row]
}

fn rows_with_date(bytes: usize) -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.date = "x".repeat(bytes);
    vec![row]
}

fn rows_with_entity(bytes: usize) -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.entity_id = Some("x".repeat(bytes));
    vec![row]
}

fn rows_with_model(bytes: usize) -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.model = Some("x".repeat(bytes));
    vec![row]
}

fn rows_with_model_group(bytes: usize) -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.model_group = Some("x".repeat(bytes));
    vec![row]
}

fn rows_with_provider(bytes: usize) -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.custom_llm_provider = Some("x".repeat(bytes));
    vec![row]
}

fn rows_with_key_index(key_index: usize) -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.key_index = key_index;
    vec![row]
}

fn duplicate_rows() -> Vec<DailyUsageRow> {
    vec![valid_row(), valid_row()]
}

fn negative_prompt_rows() -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.prompt_tokens = -1;
    vec![row]
}

fn negative_completion_rows() -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.completion_tokens = -1;
    vec![row]
}

fn negative_api_rows() -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.api_requests = -1;
    vec![row]
}

fn negative_successful_rows() -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.successful_requests = -1;
    vec![row]
}

fn negative_failed_rows() -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.failed_requests = -1;
    vec![row]
}

fn negative_spend_rows() -> Vec<DailyUsageRow> {
    let mut row = valid_row();
    row.spend = Decimal::new(-125, 2);
    vec![row]
}

#[rstest]
#[case::too_many_rows(oversized_rows(), ExpectedError::TooManyRows)]
#[case::unknown_key(rows_with_key_index(2), ExpectedError::UnknownKey)]
#[case::duplicate_row(duplicate_rows(), ExpectedError::DuplicateRow)]
#[case::long_id(rows_with_id(65), ExpectedError::TextTooLong("id"))]
#[case::long_date(rows_with_date(65), ExpectedError::TextTooLong("date"))]
#[case::long_entity(rows_with_entity(4097), ExpectedError::TextTooLong("entity_id"))]
#[case::long_model(rows_with_model(4097), ExpectedError::TextTooLong("model"))]
#[case::long_model_group(rows_with_model_group(4097), ExpectedError::TextTooLong("model_group"))]
#[case::long_provider(
    rows_with_provider(4097),
    ExpectedError::TextTooLong("custom_llm_provider")
)]
#[case::negative_prompt(
    negative_prompt_rows(),
    ExpectedError::NegativeCounter("prompt_tokens")
)]
#[case::negative_completion(
    negative_completion_rows(),
    ExpectedError::NegativeCounter("completion_tokens")
)]
#[case::negative_api(negative_api_rows(), ExpectedError::NegativeCounter("api_requests"))]
#[case::negative_successful(
    negative_successful_rows(),
    ExpectedError::NegativeCounter("successful_requests")
)]
#[case::negative_failed(
    negative_failed_rows(),
    ExpectedError::NegativeCounter("failed_requests")
)]
fn rejects_bad_usage_seed(#[case] rows: Vec<DailyUsageRow>, #[case] expected: ExpectedError) {
    let seed = usage_seed_with(rows);
    assert_usage_error(seed.validate().unwrap_err(), &expected);
}

#[test]
fn accepts_boundary_usage_seed() {
    let seed = usage_seed_with(boundary_rows());
    seed.validate().unwrap();
    let negative = usage_seed_with(negative_spend_rows());
    negative.validate().unwrap();
}

#[tokio::test]
async fn seed_revalidates_direct_callers() {
    if !postgres_available() {
        return;
    }
    let (_server, admin_url) = postgres_server().await;
    let seed = usage_seed_with(Vec::new());
    let keys = seed.keys.clone();
    let fixture = provision(&admin_url, &seed).await;
    let (client, driver) = connect(&fixture.options.url).await;
    let mut bad = valid_row();
    bad.prompt_tokens = -1;
    let rows = vec![valid_row(), bad];
    let error = daily_usage::seed(&client, &rows, &keys).await.unwrap_err();
    assert!(matches!(error, DailyUsageError::NegativeCounter { .. }));
    assert_table_state(&client, "LiteLLM_DailyUserSpend", TableState::Empty).await;
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}
