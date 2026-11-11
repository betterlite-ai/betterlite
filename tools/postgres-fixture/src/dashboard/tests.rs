#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;

use crate::dashboard::payload::JsonPayload;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls, types::FromSqlOwned};

use rstest::rstest;

use crate::connection::{PostgresFixture, RandomDatabaseName};
use crate::daily_usage::{DailyUsageRow, DailyUsageTable};
use crate::dashboard::{self, DashboardFixtureError, DashboardSeed, SpendLogSeed};
use crate::error::FixtureError;
use crate::finops::FinOpsSeed;
use crate::finops::model::{ConfigName, ConfigRow};
use crate::seed::{FixtureSeed, KeyRow, UserRow};
use crate::tests::{FixedName, postgres_server};

const LENS_TABLES: &[&str] = &["LiteLLM_Lens", "LiteLLM_LensRun", "LiteLLM_LensWorker"];

const DAILY_TABLES: &[&str] = &[
    "LiteLLM_DailyAgentSpend",
    "LiteLLM_DailyEndUserSpend",
    "LiteLLM_DailyGatewayRequests",
    "LiteLLM_DailyGlobalSpend",
    "LiteLLM_DailyGuardrailMetrics",
    "LiteLLM_DailyGuardrailUsageUnits",
    "LiteLLM_DailyModelUsage",
    "LiteLLM_DailyOrganizationSpend",
    "LiteLLM_DailyPolicyMetrics",
    "LiteLLM_DailyTagSpend",
    "LiteLLM_DailyTeamSpend",
    "LiteLLM_DailyToolSpend",
    "LiteLLM_DailyUserSpend",
];

const SPEND_COLUMNS: &[&str] = &[
    "request_id",
    "call_type",
    "api_key",
    "spend",
    "total_tokens",
    "prompt_tokens",
    "completion_tokens",
    "startTime",
    "endTime",
    "completionStartTime",
    "model",
    "model_id",
    "model_group",
    "custom_llm_provider",
    "api_base",
    "user",
    "metadata",
    "cache_hit",
    "cache_key",
    "request_tags",
    "team_id",
    "end_user",
    "requester_ip_address",
    "messages",
    "response",
    "proxy_server_request",
    "session_id",
    "status",
    "mcp_namespaced_tool_name",
    "organization_id",
    "agent_id",
    "request_duration_ms",
    "created_at",
    "updated_at",
    "litellm_call_id",
    "billing_agent_id",
];

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn spend_log(request_id: &str, key_index: usize) -> SpendLogSeed {
    SpendLogSeed {
        request_id: request_id.to_owned(),
        call_type: "acompletion".to_owned(),
        start_time: "2026-01-01T10:00:00.123".to_owned(),
        end_time: "2026-01-01T10:00:02.456".to_owned(),
        key_index,
        completion_start_time: None,
        request_duration_ms: None,
        model: None,
        model_id: None,
        model_group: None,
        custom_llm_provider: None,
        user: None,
        team_id: None,
        end_user: None,
        session_id: None,
        status: None,
        cache_hit: None,
        litellm_call_id: None,
        spend: Decimal::ZERO,
        total_tokens: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        metadata: None,
        request_tags: None,
        messages: None,
        response: None,
        proxy_server_request: None,
    }
}

fn dashboard_seed(logs: Vec<SpendLogSeed>) -> FixtureSeed {
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
                token: secret("sk-alice-token"),
                user_id: "alice".to_owned(),
                team_id: None,
                role: None,
            },
            KeyRow {
                token: secret("sk-bob-token"),
                user_id: "bob".to_owned(),
                team_id: None,
                role: None,
            },
        ],
        teams: Vec::new(),
        memberships: Vec::new(),
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
        daily_usage: None,
        finops: None,
        discovery_skills: None,
        search_tools: None,
        dashboard: Some(DashboardSeed { logs }),
        dashboard_governance: None,
    }
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

async fn scalar<T>(client: &Client, statement: &str) -> T
where
    T: FromSqlOwned,
{
    client.query_one(statement, &[]).await.unwrap().get(0)
}

async fn table_exists(client: &Client, table: &str) -> bool {
    let statement = "SELECT count(*) FROM information_schema.tables \
        WHERE table_schema = current_schema() AND table_name = $1";
    let count: i64 = client.query_one(statement, &[&table]).await.unwrap().get(0);
    count == 1
}

const COLUMN_NAMES: &str = "SELECT column_name::text FROM information_schema.columns \
    WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position";

const TABLE_NAMES: &str = "SELECT table_name::text FROM information_schema.tables \
    WHERE table_schema = current_schema() AND table_name LIKE $1 ORDER BY table_name";

async fn metadata_names(client: &Client, statement: &str, pattern: &str) -> Vec<String> {
    client
        .query(statement, &[&pattern])
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

async fn schema_fingerprint(client: &Client) -> Vec<String> {
    let statement = "SELECT table_name::text || '.' || column_name::text \
        FROM information_schema.columns \
        WHERE table_schema = current_schema() ORDER BY table_name, ordinal_position";
    client
        .query(statement, &[])
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

async fn database_exists(admin_url: &SecretString, database: &str) -> bool {
    let (client, driver) = connect(admin_url).await;
    let statement = "SELECT 1 FROM pg_database WHERE datname = $1";
    let exists = client
        .query_opt(statement, &[&database])
        .await
        .unwrap()
        .is_some();
    disconnect(client, driver).await;
    exists
}

fn owned_names(tables: &[&str]) -> Vec<String> {
    tables.iter().map(|name| (*name).to_owned()).collect()
}

async fn assert_lens_tables(client: &Client, present: bool) {
    for table in LENS_TABLES {
        assert_eq!(
            table_exists(client, table).await,
            present,
            "{table} presence in the dashboard fixture"
        );
    }
}

async fn assert_historical_schema(client: &Client) {
    assert_lens_tables(client, false).await;
    let historical = metadata_names(client, COLUMN_NAMES, "LiteLLM_SpendLogs").await;
    assert!(
        !historical.iter().any(|name| name == "request_duration_ms"),
        "the baseline fixture keeps the historical SpendLogs shape"
    );
}

async fn assert_current_schema(client: &Client) {
    assert_lens_tables(client, true).await;
    // the ROI calculator reads the daily user spend and user tables through its own ports
    assert!(table_exists(client, "LiteLLM_DailyUserSpend").await);
    assert!(table_exists(client, "LiteLLM_UserTable").await);
    assert_eq!(
        metadata_names(client, TABLE_NAMES, "LiteLLM\\_Daily%").await,
        owned_names(DAILY_TABLES)
    );
    assert_eq!(
        metadata_names(client, COLUMN_NAMES, "LiteLLM_SpendLogs").await,
        owned_names(SPEND_COLUMNS)
    );
    assert_eq!(
        scalar::<i64>(client, "SELECT count(*) FROM \"LiteLLM_SpendLogs\"").await,
        0,
        "an empty dashboard seed installs the schema without inventing rows"
    );
}

fn assert_confined_filenames() {
    let sql_root = Path::new("/pinned/sql");
    super::schema::confined_path(sql_root, "20250326162113_baseline.sql").unwrap();
    for name in ["../escape.sql", "nested/file.sql", "/absolute.sql", ".."] {
        assert!(
            matches!(
                super::schema::confined_path(sql_root, name),
                Err(DashboardFixtureError::UnsafeFilename { .. })
            ),
            "{name} must stay outside the pinned sql directory"
        );
    }
}

async fn assert_schema_selection(admin_url: &SecretString) {
    assert_confined_filenames();
    let baseline = provision(admin_url, &FixtureSeed::default()).await;
    let (baseline_client, baseline_driver) = connect(&baseline.options.url).await;
    assert_historical_schema(&baseline_client).await;
    disconnect(baseline_client, baseline_driver).await;
    baseline.close_async().await.unwrap();

    let empty = provision(admin_url, &dashboard_seed(Vec::new())).await;
    let (empty_client, empty_driver) = connect(&empty.options.url).await;
    assert_current_schema(&empty_client).await;
    disconnect(empty_client, empty_driver).await;
    empty.close_async().await.unwrap();
}

async fn assert_same_schema(first: &Client, second: &Client) {
    assert_eq!(
        schema_fingerprint(first).await,
        schema_fingerprint(second).await,
        "two dashboard fixtures carry the same schema"
    );
}

async fn assert_independent_lens_rows(first: &Client, second: &Client) {
    first
        .batch_execute(
            "INSERT INTO \"LiteLLM_Lens\" (\"id\",\"version\",\"data\") VALUES ('lens-one',1,'{}')",
        )
        .await
        .unwrap();
    assert_eq!(
        scalar::<i64>(first, "SELECT count(*) FROM \"LiteLLM_Lens\"").await,
        1
    );
    assert_eq!(
        scalar::<i64>(second, "SELECT count(*) FROM \"LiteLLM_Lens\"").await,
        0,
        "two dashboard fixtures keep independent data"
    );
}

async fn assert_dashboard_fixtures_are_independent(admin_url: &SecretString) {
    let first = provision(admin_url, &dashboard_seed(Vec::new())).await;
    let second = provision(admin_url, &dashboard_seed(Vec::new())).await;
    let (first_client, first_driver) = connect(&first.options.url).await;
    let (second_client, second_driver) = connect(&second.options.url).await;
    assert_same_schema(&first_client, &second_client).await;
    assert_independent_lens_rows(&first_client, &second_client).await;
    disconnect(first_client, first_driver).await;
    disconnect(second_client, second_driver).await;
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
}

#[tokio::test]
async fn dashboard_schema_is_independent() {
    let (_server, admin_url) = postgres_server().await;
    assert_schema_selection(&admin_url).await;
    assert_dashboard_fixtures_are_independent(&admin_url).await;
}

const STORED_ROW: &str = "SELECT concat_ws('|', \
    \"call_type\", \"api_key\", \"spend\"::text, \
    \"total_tokens\"::text, \"prompt_tokens\"::text, \"completion_tokens\"::text, \
    to_char(\"startTime\", 'YYYY-MM-DD HH24:MI:SS.MS'), \
    to_char(\"endTime\", 'YYYY-MM-DD HH24:MI:SS.MS'), \
    coalesce(to_char(\"completionStartTime\", 'YYYY-MM-DD HH24:MI:SS.MS'), '<null>'), \
    coalesce(\"request_duration_ms\"::text, '<null>'), \
    coalesce(\"model\", '<null>'), coalesce(\"model_id\", '<null>'), \
    coalesce(\"model_group\", '<null>'), coalesce(\"custom_llm_provider\", '<null>'), \
    coalesce(\"user\", '<null>'), coalesce(\"team_id\", '<null>'), \
    coalesce(\"end_user\", '<null>'), coalesce(\"session_id\", '<null>'), \
    coalesce(\"status\", '<null>'), coalesce(\"cache_hit\", '<null>'), \
    coalesce(\"litellm_call_id\", '<null>'), \
    coalesce(\"metadata\"::text, '<null>'), coalesce(\"request_tags\"::text, '<null>'), \
    coalesce(\"messages\"::text, '<null>'), coalesce(\"response\"::text, '<null>'), \
    coalesce(\"proxy_server_request\"::text, '<null>')) \
    FROM \"LiteLLM_SpendLogs\" WHERE \"request_id\" = $1";

async fn stored_row(client: &Client, request_id: &str) -> String {
    client
        .query_one(STORED_ROW, &[&request_id])
        .await
        .unwrap()
        .get(0)
}

fn alice_payloads(alice: &mut SpendLogSeed) {
    alice.request_tags = Some(JsonPayload::Array(vec![
        JsonPayload::Text("tag-a".to_owned()),
        JsonPayload::Text("tag-b".to_owned()),
    ]));
    alice.messages = Some(JsonPayload::Array(vec![JsonPayload::Object(
        BTreeMap::from([
            ("role".to_owned(), JsonPayload::Text("user".to_owned())),
            ("content".to_owned(), JsonPayload::Text("hello".to_owned())),
        ]),
    )]));
    alice.response = Some(JsonPayload::Object(BTreeMap::from([(
        "id".to_owned(),
        JsonPayload::Text("chatcmpl-1".to_owned()),
    )])));
    alice.proxy_server_request = Some(JsonPayload::Object(BTreeMap::from([(
        "url".to_owned(),
        JsonPayload::Text("http://localhost/v1/chat/completions".to_owned()),
    )])));
}

fn round_trip_logs() -> Vec<SpendLogSeed> {
    let mut alice = spend_log("req-alice", 0);
    alice.completion_start_time = Some("2026-01-01T10:00:01.000".to_owned());
    alice.request_duration_ms = Some(2333_i32);
    alice.model = Some("openai/gpt-5".to_owned());
    alice.model_id = Some("model-id".to_owned());
    alice.model_group = Some("default".to_owned());
    alice.custom_llm_provider = Some("openai".to_owned());
    alice.user = Some("alice".to_owned());
    alice.team_id = Some("team-red".to_owned());
    alice.end_user = Some("customer".to_owned());
    alice.session_id = Some("session-shared".to_owned());
    alice.status = Some("success".to_owned());
    alice.cache_hit = Some("True".to_owned());
    alice.litellm_call_id = Some("call-alice".to_owned());
    alice.spend = Decimal::new(125, 2);
    alice.total_tokens = 100_i32;
    alice.prompt_tokens = 60_i32;
    alice.completion_tokens = 40_i32;
    alice_payloads(&mut alice);

    let mut bob = spend_log("req-bob", 1);
    bob.start_time = "2026-01-01T11:00:00.000".to_owned();
    bob.end_time = "2026-01-01T11:00:01.000".to_owned();
    bob.session_id = Some("session-shared".to_owned());
    bob.spend = Decimal::new(-50, 2);
    bob.metadata = Some(JsonPayload::Object(BTreeMap::from([(
        "project_id".to_owned(),
        JsonPayload::Text("project-x".to_owned()),
    )])));

    let mut minimal = spend_log("req-minimal", 1);
    minimal.call_type = "aembedding".to_owned();
    minimal.start_time = "2026-01-02T00:00:00".to_owned();
    minimal.end_time = "2026-01-02T00:00:01".to_owned();
    vec![alice, bob, minimal]
}

const ALICE_ROW: &str = r#"acompletion|<api-key>|1.25|100|60|40|2026-01-01 10:00:00.123|2026-01-01 10:00:02.456|2026-01-01 10:00:01.000|2333|openai/gpt-5|model-id|default|openai|alice|team-red|customer|session-shared|success|True|call-alice|{}|["tag-a", "tag-b"]|[{"role": "user", "content": "hello"}]|{"id": "chatcmpl-1"}|{"url": "http://localhost/v1/chat/completions"}"#;

const BOB_ROW: &str = r#"acompletion|<api-key>|-0.5|0|0|0|2026-01-01 11:00:00.000|2026-01-01 11:00:01.000|<null>|<null>||<null>|<null>|<null>|<null>|<null>|<null>|session-shared|<null>|<null>|<null>|{"project_id": "project-x"}|[]|{}|{}|{}"#;

const MINIMAL_ROW: &str = "aembedding|<api-key>|0|0|0|0|2026-01-02 00:00:00.000|2026-01-02 00:00:01.000|<null>|<null>||<null>|<null>|<null>|<null>|<null>|<null>|<null>|<null>|<null>|<null>|{}|[]|{}|{}|{}";

fn expected_row(template: &str, token: &str) -> String {
    template.replace("<api-key>", &FixtureSeed::key_hash(token))
}

async fn assert_round_trip_rows(client: &Client) {
    let alice = stored_row(client, "req-alice").await;
    assert_eq!(alice, expected_row(ALICE_ROW, "sk-alice-token"));
    assert!(alice.contains(&FixtureSeed::key_hash("sk-alice-token")));
    assert!(!alice.contains("sk-alice-token"));
    assert_eq!(
        stored_row(client, "req-bob").await,
        expected_row(BOB_ROW, "sk-bob-token")
    );
    assert_eq!(
        stored_row(client, "req-minimal").await,
        expected_row(MINIMAL_ROW, "sk-bob-token")
    );
}

async fn assert_shared_session(client: &Client) {
    let key_hashes = client
        .query(
            "SELECT \"api_key\" FROM \"LiteLLM_SpendLogs\" \
             WHERE \"request_id\" IN ('req-alice','req-bob') ORDER BY \"request_id\"",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>();
    assert_eq!(
        key_hashes,
        [
            FixtureSeed::key_hash("sk-alice-token"),
            FixtureSeed::key_hash("sk-bob-token")
        ],
        "two users' logs keep distinct hashed key references"
    );
    assert_eq!(
        scalar::<i64>(
            client,
            "SELECT count(*) FROM \"LiteLLM_SpendLogs\" WHERE \"session_id\" = 'session-shared'",
        )
        .await,
        2,
        "the shared session joins both users' logs"
    );
}

async fn assert_duplicate_request_id_fails(client: &Client, seed: &FixtureSeed) {
    let dashboard = seed.dashboard.as_ref().unwrap();
    let error = dashboard::seed(client, dashboard, seed).await.unwrap_err();
    assert!(
        matches!(error, DashboardFixtureError::Insert { .. }),
        "a duplicate request_id is an insert failure, not an upsert: {error:?}"
    );
    assert_eq!(
        scalar::<i64>(client, "SELECT count(*) FROM \"LiteLLM_SpendLogs\"").await,
        3
    );
    assert_eq!(
        stored_row(client, "req-alice").await,
        expected_row(ALICE_ROW, "sk-alice-token")
    );
}

#[tokio::test]
async fn dashboard_spend_rows_round_trip() {
    let (_server, admin_url) = postgres_server().await;
    let seed = dashboard_seed(round_trip_logs());
    let fixture = provision(&admin_url, &seed).await;
    let (client, driver) = connect(&fixture.options.url).await;
    assert_round_trip_rows(&client).await;
    assert_shared_session(&client).await;
    assert_duplicate_request_id_fails(&client, &seed).await;
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedError {
    Accepted,
    Redacted,
    TooManyLogs,
    DuplicateRequestId,
    UnknownKey,
    Text(&'static str),
    Json(&'static str),
    Counter(&'static str),
    Duration,
}

fn assert_dashboard_error(error: &DashboardFixtureError, expected: &ExpectedError) {
    let matched = match (error, expected) {
        (DashboardFixtureError::TooManyLogs { .. }, ExpectedError::TooManyLogs)
        | (DashboardFixtureError::DuplicateRequestId { .. }, ExpectedError::DuplicateRequestId)
        | (DashboardFixtureError::UnknownKey { .. }, ExpectedError::UnknownKey)
        | (DashboardFixtureError::NegativeDuration { .. }, ExpectedError::Duration) => true,
        (DashboardFixtureError::TextTooLong { field, .. }, ExpectedError::Text(wanted))
        | (DashboardFixtureError::JsonTooLong { field, .. }, ExpectedError::Json(wanted))
        | (DashboardFixtureError::NegativeCounter { field, .. }, ExpectedError::Counter(wanted)) => {
            field == wanted
        }
        _ => false,
    };
    assert!(matched, "unexpected dashboard error {error:?}");
}

fn dashboard_error(seed: &FixtureSeed) -> DashboardFixtureError {
    let FixtureError::Dashboard(source) = seed.validate().unwrap_err() else {
        panic!("expected a dashboard fixture error");
    };
    source
}

fn excess_logs() -> Vec<SpendLogSeed> {
    (0..=256)
        .map(|index| spend_log(&format!("req-{index}"), 0))
        .collect()
}

fn duplicate_id_logs() -> Vec<SpendLogSeed> {
    vec![spend_log("req-same", 0), spend_log("req-same", 1)]
}

fn unknown_key_logs() -> Vec<SpendLogSeed> {
    vec![spend_log("req-key", 2)]
}

fn request_id_bytes(bytes: usize) -> Vec<SpendLogSeed> {
    let mut log = spend_log("req", 0);
    log.request_id = "x".repeat(bytes);
    vec![log]
}

fn model_bytes(bytes: usize) -> Vec<SpendLogSeed> {
    let mut log = spend_log("req", 0);
    log.model = Some("x".repeat(bytes));
    vec![log]
}

fn completion_start_bytes(bytes: usize) -> Vec<SpendLogSeed> {
    let mut log = spend_log("req", 0);
    log.completion_start_time = Some("x".repeat(bytes));
    vec![log]
}

fn metadata_json(bytes: usize) -> Vec<SpendLogSeed> {
    let mut log = spend_log("req", 0);
    log.metadata = Some(JsonPayload::Text("x".repeat(bytes)));
    vec![log]
}

fn request_tags_json(bytes: usize) -> Vec<SpendLogSeed> {
    let mut log = spend_log("req", 0);
    log.request_tags = Some(JsonPayload::Text("x".repeat(bytes)));
    vec![log]
}

fn negative_counter(field: &'static str) -> Vec<SpendLogSeed> {
    let mut log = spend_log("req", 0);
    match field {
        "total_tokens" => log.total_tokens = -1_i32,
        "prompt_tokens" => log.prompt_tokens = -1_i32,
        _ => log.completion_tokens = -1_i32,
    }
    vec![log]
}

fn negative_duration() -> Vec<SpendLogSeed> {
    let mut log = spend_log("req", 0);
    log.request_duration_ms = Some(-1_i32);
    vec![log]
}

fn boundary_logs() -> Vec<SpendLogSeed> {
    let mut logs = (0_usize..256)
        .map(|index| spend_log(&format!("req-{index}"), index & 1))
        .collect::<Vec<_>>();
    let first = logs.first_mut().expect("boundary logs are not empty");
    first.request_id = "i".repeat(0x1000);
    first.model = Some("m".repeat(0x1000));
    first.completion_start_time = Some("c".repeat(0x1000));
    first.metadata = Some(JsonPayload::Text("x".repeat(0xFFFE)));
    first.request_duration_ms = Some(0_i32);
    first.spend = Decimal::new(-125, 2);
    logs
}

fn redacted_logs() -> Vec<SpendLogSeed> {
    let mut first = spend_log("req-secret", 0);
    first.metadata = Some(JsonPayload::Object(BTreeMap::from([(
        "payload".to_owned(),
        JsonPayload::Text("super-secret-payload".to_owned()),
    )])));
    let mut second = spend_log("req-oversized", 0);
    second.metadata = Some(JsonPayload::Text("x".repeat(0xFFFF)));
    vec![first, second]
}

fn assert_seed_outcome(seed: &FixtureSeed, expected: ExpectedError) {
    match expected {
        ExpectedError::Accepted => seed.validate().unwrap(),
        ExpectedError::Redacted => {
            let message = dashboard_error(seed).to_string();
            assert!(message.contains("row 1"), "{message}");
            assert!(message.contains("metadata"), "{message}");
            assert!(!message.contains("super-secret-payload"), "{message}");
            assert!(!message.contains("sk-alice-token"), "{message}");
        }
        ExpectedError::TooManyLogs
        | ExpectedError::DuplicateRequestId
        | ExpectedError::UnknownKey
        | ExpectedError::Text(_)
        | ExpectedError::Json(_)
        | ExpectedError::Counter(_)
        | ExpectedError::Duration => {
            assert_dashboard_error(&dashboard_error(seed), &expected);
        }
    }
}

#[rstest]
#[case::too_many_logs(excess_logs(), ExpectedError::TooManyLogs)]
#[case::duplicate_request_id(duplicate_id_logs(), ExpectedError::DuplicateRequestId)]
#[case::unknown_key(unknown_key_logs(), ExpectedError::UnknownKey)]
#[case::long_request_id(request_id_bytes(0x1001), ExpectedError::Text("request_id"))]
#[case::long_model(model_bytes(0x1001), ExpectedError::Text("model"))]
#[case::long_completion_start(
    completion_start_bytes(0x1001),
    ExpectedError::Text("completion_start_time")
)]
#[case::oversized_metadata(metadata_json(0xFFFF), ExpectedError::Json("metadata"))]
#[case::oversized_request_tags(request_tags_json(0xFFFF), ExpectedError::Json("request_tags"))]
#[case::negative_total_tokens(
    negative_counter("total_tokens"),
    ExpectedError::Counter("total_tokens")
)]
#[case::negative_prompt_tokens(
    negative_counter("prompt_tokens"),
    ExpectedError::Counter("prompt_tokens")
)]
#[case::negative_completion_tokens(
    negative_counter("completion_tokens"),
    ExpectedError::Counter("completion_tokens")
)]
#[case::negative_duration(negative_duration(), ExpectedError::Duration)]
#[case::boundary_rows(boundary_logs(), ExpectedError::Accepted)]
#[case::redacted_diagnostics(redacted_logs(), ExpectedError::Redacted)]
fn dashboard_seed_rejects_bad_rows(
    #[case] logs: Vec<SpendLogSeed>,
    #[case] expected: ExpectedError,
) {
    assert_seed_outcome(&dashboard_seed(logs), expected);
}

#[tokio::test]
async fn dashboard_seed_rejects_bad_rows_before_creating_a_database() {
    let (_server, admin_url) = postgres_server().await;
    let seed = dashboard_seed(unknown_key_logs());
    let mut generator = FixedName("dashboard_bad_seed".to_owned());
    let result =
        PostgresFixture::provision_with_name_generator(Some(&admin_url), &seed, &mut generator)
            .resolve()
            .await;
    assert!(matches!(
        result,
        Err(FixtureError::Dashboard(
            DashboardFixtureError::UnknownKey { .. }
        ))
    ));
    assert!(
        !database_exists(&admin_url, "dashboard_bad_seed").await,
        "validation runs before the database is created"
    );
}

#[tokio::test]
async fn dashboard_seed_rejects_bad_rows_and_cleans_up_insert_failures() {
    let (_server, admin_url) = postgres_server().await;
    let mut log = spend_log("req-time", 0);
    log.start_time = "not-a-timestamp".to_owned();
    let seed = dashboard_seed(vec![log]);
    let mut generator = FixedName("dashboard_bad_timestamp".to_owned());
    let result =
        PostgresFixture::provision_with_name_generator(Some(&admin_url), &seed, &mut generator)
            .resolve()
            .await;
    assert!(matches!(
        result,
        Err(FixtureError::Dashboard(
            DashboardFixtureError::Insert { .. }
        ))
    ));
    assert!(
        !database_exists(&admin_url, "dashboard_bad_timestamp").await,
        "a failed insert cleans the database up through the fixture owner"
    );
}

fn daily_usage_row() -> DailyUsageRow {
    DailyUsageRow {
        table: DailyUsageTable::User,
        id: "daily-row".to_owned(),
        entity_id: Some("alice".to_owned()),
        date: "2025-01-01".to_owned(),
        key_index: 0,
        model: Some("openai/gpt-5".to_owned()),
        custom_llm_provider: Some("openai".to_owned()),
        model_group: Some("default".to_owned()),
        spend: Decimal::new(125, 2),
        prompt_tokens: 10,
        completion_tokens: 5,
        api_requests: 1,
        successful_requests: 1,
        failed_requests: 0,
    }
}

fn interop_seed() -> FixtureSeed {
    let mut seed = dashboard_seed(vec![spend_log("req-dashboard", 0)]);
    seed.daily_usage = Some(vec![daily_usage_row()]);
    seed.finops = Some(FinOpsSeed {
        organizations: Vec::new(),
        key_metadata: Vec::new(),
        daily_usage: Vec::new(),
        config_rows: vec![ConfigRow {
            name: ConfigName::CloudzeroSettings,
            serialized_value: None,
        }],
    });
    seed
}

async fn assert_combined_seed(client: &Client) -> Vec<String> {
    assert_eq!(
        scalar::<i64>(client, "SELECT count(*) FROM \"LiteLLM_SpendLogs\"").await,
        1,
        "the dashboard seed survives the daily and finops extensions"
    );
    let rows = client
        .query(
            "SELECT \"id\" FROM \"LiteLLM_DailyUserSpend\" ORDER BY \"id\"",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>();
    assert_eq!(rows, ["daily-row"]);
    assert_eq!(
        scalar::<i64>(
            client,
            "SELECT count(*) FROM \"LiteLLM_Config\" WHERE \"param_name\" = 'cloudzero_settings'",
        )
        .await,
        1,
        "the finops config row survives the dashboard schema"
    );
    schema_fingerprint(client).await
}

#[tokio::test]
async fn dashboard_daily_seed_interop() {
    let (_server, admin_url) = postgres_server().await;

    let combined = provision(&admin_url, &interop_seed()).await;
    let (client, driver) = connect(&combined.options.url).await;
    let combined_fingerprint = assert_combined_seed(&client).await;
    disconnect(client, driver).await;
    combined.close_async().await.unwrap();

    let dashboard_only = provision(&admin_url, &dashboard_seed(Vec::new())).await;
    let (only_client, only_driver) = connect(&dashboard_only.options.url).await;
    assert_eq!(
        schema_fingerprint(&only_client).await,
        combined_fingerprint,
        "the daily and finops extensions do not create schema a second time"
    );
    disconnect(only_client, only_driver).await;
    dashboard_only.close_async().await.unwrap();
}
