use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::types::{FromSqlOwned, ToSql};
use tokio_postgres::{Client, NoTls};

use crate::connection::{PostgresFixture, RandomDatabaseName};
use crate::dashboard_governance::DashboardGovernanceSeed;
use crate::seed::FixtureSeed;
use crate::tests::postgres_server;

use super::support::{empty_dashboard, sample_seed, seed_with};

const CREATED: &str = "2026-01-01 00:00:00.000";

const METRIC_STATE: &str = "SELECT concat_ws('|', \"guardrail_id\", \"date\", \"requests_evaluated\"::text, \"passed_count\"::text, \"blocked_count\"::text, \"flagged_count\"::text, coalesce(\"avg_score\"::text, '<null>'), coalesce(\"avg_latency_ms\"::text, '<null>'), to_char(\"created_at\", 'YYYY-MM-DD HH24:MI:SS.MS'), to_char(\"updated_at\", 'YYYY-MM-DD HH24:MI:SS.MS')) FROM \"LiteLLM_DailyGuardrailMetrics\" ORDER BY \"guardrail_id\"";

const USAGE_STATE: &str = "SELECT concat_ws('|', \"guardrail_id\", \"date\", \"team_id\", \"api_key\", \"usage_unit\", \"units\"::text, coalesce(\"cost\"::text, '<null>'), \"untracked_units\"::text, to_char(\"created_at\", 'YYYY-MM-DD HH24:MI:SS.MS'), to_char(\"updated_at\", 'YYYY-MM-DD HH24:MI:SS.MS')) FROM \"LiteLLM_DailyGuardrailUsageUnits\" ORDER BY \"usage_unit\"";

const GUARDRAIL_INDEX_STATE: &str = "SELECT concat_ws('|', \"request_id\", \"guardrail_id\", coalesce(\"policy_id\", '<null>'), to_char(\"start_time\", 'YYYY-MM-DD HH24:MI:SS.MS')) FROM \"LiteLLM_SpendLogGuardrailIndex\" ORDER BY \"guardrail_id\"";

const TOOL_INDEX_STATE: &str = "SELECT concat_ws('|', \"request_id\", \"tool_name\", to_char(\"start_time\", 'YYYY-MM-DD HH24:MI:SS.MS')) FROM \"LiteLLM_SpendLogToolIndex\" ORDER BY \"tool_name\"";

const TOOL_STATE: &str = "SELECT concat_ws('|', \"tool_id\", \"tool_name\", coalesce(\"origin\", '<null>'), \"input_policy\", \"output_policy\", \"call_count\"::text, coalesce(\"key_hash\", '<null>'), coalesce(\"team_id\", '<null>'), coalesce(\"key_alias\", '<null>'), coalesce(\"user_agent\", '<null>'), coalesce(to_char(\"last_used_at\", 'YYYY-MM-DD HH24:MI:SS.MS'), '<null>'), to_char(\"created_at\", 'YYYY-MM-DD HH24:MI:SS.MS'), to_char(\"updated_at\", 'YYYY-MM-DD HH24:MI:SS.MS')) FROM \"LiteLLM_ToolTable\" ORDER BY \"tool_id\"";

const USER_TEAMS_STATE: &str = "SELECT concat_ws('|', \"user_id\", array_to_string(\"teams\", ',')) FROM \"LiteLLM_UserTable\" ORDER BY \"user_id\"";

const LOG_METADATA_STATE: &str = "SELECT concat_ws('|', \"status\", \"metadata\" #>> '{guardrail_information,0,guardrail_id}', \"metadata\" #>> '{guardrail_information,0,guardrail_name}', \"metadata\" #>> '{guardrail_information,0,guardrail_provider}', \"metadata\" #>> '{guardrail_information,0,guardrail_mode}', \"metadata\" #>> '{guardrail_information,0,guardrail_status}', \"metadata\" #>> '{guardrail_information,0,duration}', \"metadata\" #>> '{guardrail_information,0,confidence_score}', \"metadata\" #>> '{guardrail_information,0,risk_score}', \"metadata\" #>> '{guardrail_information,0,guardrail_response,action}', \"messages\" #>> '{0,content}', \"response\" #>> '{id}', \"proxy_server_request\" #>> '{url}') FROM \"LiteLLM_SpendLogs\" WHERE \"request_id\" = 'req-blocked'";

const COLUMN_NAMES: &str = "SELECT column_name::text FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1";

const GOVERNANCE_TABLES: &str = "SELECT table_name::text FROM information_schema.tables WHERE table_schema = current_schema() AND table_name IN ('LiteLLM_DailyGuardrailMetrics', 'LiteLLM_ToolTable') ORDER BY table_name";

const SCHEMA: &[(&str, &[&str])] = &[
    (
        "LiteLLM_GuardrailsTable",
        &[
            "guardrail_id",
            "guardrail_name",
            "litellm_params",
            "guardrail_info",
            "team_id",
            "status",
            "submitted_at",
            "reviewed_at",
        ],
    ),
    (
        "LiteLLM_PolicyTable",
        &[
            "policy_id",
            "policy_name",
            "version_number",
            "version_status",
            "is_latest",
            "inherit",
            "guardrails_add",
            "guardrails_remove",
            "condition",
            "pipeline",
        ],
    ),
    (
        "LiteLLM_PolicyAttachmentTable",
        &[
            "attachment_id",
            "policy_name",
            "scope",
            "teams",
            "keys",
            "models",
            "tags",
            "priority",
            "is_default",
        ],
    ),
    (
        "LiteLLM_ToolTable",
        &[
            "tool_id",
            "tool_name",
            "origin",
            "input_policy",
            "output_policy",
            "call_count",
            "key_hash",
            "team_id",
            "key_alias",
            "user_agent",
            "last_used_at",
        ],
    ),
    (
        "LiteLLM_DailyGuardrailMetrics",
        &[
            "guardrail_id",
            "date",
            "requests_evaluated",
            "passed_count",
            "blocked_count",
            "flagged_count",
            "avg_score",
            "avg_latency_ms",
        ],
    ),
    (
        "LiteLLM_DailyGuardrailUsageUnits",
        &[
            "guardrail_id",
            "date",
            "team_id",
            "api_key",
            "usage_unit",
            "units",
            "cost",
            "untracked_units",
        ],
    ),
    (
        "LiteLLM_SpendLogGuardrailIndex",
        &["request_id", "guardrail_id", "policy_id", "start_time"],
    ),
    (
        "LiteLLM_SpendLogToolIndex",
        &["request_id", "tool_name", "start_time"],
    ),
    (
        "LiteLLM_ObjectPermissionTable",
        &[
            "object_permission_id",
            "mcp_servers",
            "blocked_tools",
            "mcp_toolsets",
            "search_tools",
        ],
    ),
    (
        "LiteLLM_UserTable",
        &["user_id", "teams", "object_permission_id"],
    ),
    (
        "LiteLLM_VerificationToken",
        &["token", "key_alias", "object_permission_id"],
    ),
    (
        "LiteLLM_TeamTable",
        &["team_id", "team_alias", "object_permission_id"],
    ),
];

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

async fn strings(client: &Client, statement: &str, params: &[&(dyn ToSql + Sync)]) -> Vec<String> {
    client
        .query(statement, params)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

async fn assert_columns(client: &Client, table: &str, expected: &[&str]) {
    let present = strings(client, COLUMN_NAMES, &[&table]).await;
    assert!(
        !present.is_empty(),
        "{table} is missing from the pinned schema"
    );
    for column in expected {
        assert!(
            present.iter().any(|name| name == column),
            "{table} needs {column}, has {present:?}"
        );
    }
}

async fn assert_governance_schema(client: &Client) {
    for (table, columns) in SCHEMA {
        assert_columns(client, table, columns).await;
    }
}

fn expected_state() -> Vec<String> {
    let alice = FixtureSeed::key_hash("sk-alice-token");
    vec![
        format!("gr-config|2026-01-04|3|3|0|0|<null>|<null>|{CREATED}|{CREATED}"),
        format!("gr-pii|2026-01-04|10|7|2|1|0.875|12.5|{CREATED}|{CREATED}"),
        format!(
            "gr-pii|2026-01-04|team-red|{alice}|contentPolicyUnits|5|0.25|0|{CREATED}|{CREATED}"
        ),
        format!("gr-pii|2026-01-04|||textUnits|3|<null>|3|{CREATED}|{CREATED}"),
        "req-blocked|gr-config|<null>|2026-01-04 10:00:00.000".to_owned(),
        "req-blocked|gr-pii|policy-eu|2026-01-04 10:00:00.000".to_owned(),
        "req-tools|mcp__github/search|2026-01-04 11:00:00.000".to_owned(),
        format!(
            "tool-github|mcp__github/search|user_defined|blocked|trusted|42|{alice}|team-red|alice-discovery-key|openai-python/1.0|2026-01-04 11:00:01.000|{CREATED}|{CREATED}"
        ),
        format!(
            "tool-weather|get_weather|<null>|untrusted|untrusted|0|<null>|<null>|<null>|<null>|<null>|{CREATED}|{CREATED}"
        ),
        "alice|team-blue,team-red".to_owned(),
        "bob|".to_owned(),
    ]
}

async fn governance_state(client: &Client) -> Vec<String> {
    let mut state = strings(client, METRIC_STATE, &[]).await;
    state.extend(strings(client, USAGE_STATE, &[]).await);
    state.extend(strings(client, GUARDRAIL_INDEX_STATE, &[]).await);
    state.extend(strings(client, TOOL_INDEX_STATE, &[]).await);
    state.extend(strings(client, TOOL_STATE, &[]).await);
    state.extend(strings(client, USER_TEAMS_STATE, &[]).await);
    state
}

async fn assert_governance_rows(client: &Client) {
    assert_eq!(
        governance_state(client).await,
        expected_state(),
        "the governance rows keep their counters, null costs, ownership hashes, teams and timestamps"
    );
}

async fn assert_page_data(client: &Client) {
    let alice = FixtureSeed::key_hash("sk-alice-token");
    let alias: Option<String> = client
        .query_one(
            "SELECT \"key_alias\" FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = $1",
            &[&alice],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(alias.as_deref(), Some("alice-live-key"));
    let raw: i64 = scalar(
        client,
        "SELECT count(*) FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = 'sk-alice-token'",
    )
    .await;
    assert_eq!(raw, 0, "the fixture stores key hashes, never the tokens");
    let unrelated = FixtureSeed::key_hash("sk-unrelated-token");
    let tool_hashes = strings(
        client,
        "SELECT coalesce(\"key_hash\", '<null>') FROM \"LiteLLM_ToolTable\" ORDER BY \"tool_id\"",
        &[],
    )
    .await;
    assert!(
        !tool_hashes.iter().any(|hash| hash == &unrelated),
        "an unrelated key's hash never reaches the tool rows"
    );
    assert_eq!(
        strings(client, LOG_METADATA_STATE, &[]).await,
        [
            "failure|gr-pii|pii-guard|bedrock|pre_call|guardrail_intervened|0.25|0.97|8.5|BLOCKED|hello|chatcmpl-blocked|http://localhost/v1/chat/completions"
        ],
        "the blocked log keeps the metadata, messages, response and request the drawer reads"
    );
}

async fn tool_call_count(client: &Client) -> String {
    scalar(
        client,
        "SELECT \"call_count\"::text FROM \"LiteLLM_ToolTable\" WHERE \"tool_id\" = 'tool-github'",
    )
    .await
}

#[tokio::test]
async fn governance_schema_and_rows() {
    let (_server, admin_url) = postgres_server().await;
    let fixture = provision(&admin_url, &sample_seed()).await;
    let (client, driver) = connect(&fixture.options.url).await;
    assert_governance_schema(&client).await;
    assert_governance_rows(&client).await;
    assert_page_data(&client).await;
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn governance_databases_are_independent() {
    let (_server, admin_url) = postgres_server().await;
    let seed = sample_seed();
    let first = provision(&admin_url, &seed).await;
    let second = provision(&admin_url, &seed).await;
    let (first_client, first_driver) = connect(&first.options.url).await;
    let (second_client, second_driver) = connect(&second.options.url).await;
    assert_ne!(
        scalar::<String>(&first_client, "SELECT current_database()").await,
        scalar::<String>(&second_client, "SELECT current_database()").await,
        "each gateway gets its own fixture database"
    );
    assert_eq!(
        governance_state(&first_client).await,
        governance_state(&second_client).await,
        "both databases receive identical rows, ids and timestamps"
    );
    first_client
        .execute(
            "UPDATE \"LiteLLM_ToolTable\" SET \"call_count\" = 99 WHERE \"tool_id\" = 'tool-github'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(tool_call_count(&first_client).await, "99");
    assert_eq!(tool_call_count(&second_client).await, "42");
    disconnect(first_client, first_driver).await;
    disconnect(second_client, second_driver).await;
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
}

async fn tables_and_teams(
    admin_url: &SecretString,
    seed: &FixtureSeed,
) -> (Vec<String>, Vec<String>) {
    let fixture = provision(admin_url, seed).await;
    let (client, driver) = connect(&fixture.options.url).await;
    let tables = strings(&client, GOVERNANCE_TABLES, &[]).await;
    let teams = strings(&client, USER_TEAMS_STATE, &[]).await;
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
    (tables, teams)
}

async fn assert_without_governance(admin_url: &SecretString) {
    let (bare_tables, bare_teams) = tables_and_teams(admin_url, &seed_with(None, None)).await;
    assert!(
        bare_tables.is_empty(),
        "without dashboard, no governance table exists: {bare_tables:?}"
    );
    assert_eq!(
        bare_teams,
        ["alice|", "bob|"],
        "without governance nothing rewrites the user teams"
    );
    let dashboard = seed_with(Some(empty_dashboard()), None);
    let (dashboard_tables, dashboard_teams) = tables_and_teams(admin_url, &dashboard).await;
    assert_eq!(
        dashboard_tables,
        ["LiteLLM_DailyGuardrailMetrics", "LiteLLM_ToolTable"]
    );
    assert_eq!(
        dashboard_teams,
        ["alice|", "bob|"],
        "governance absent leaves the user teams alone"
    );
}

async fn assert_empty_governance(admin_url: &SecretString) {
    let empty = DashboardGovernanceSeed::default();
    let fixture = provision(admin_url, &seed_with(Some(empty_dashboard()), Some(empty))).await;
    let (client, driver) = connect(&fixture.options.url).await;
    for table in [
        "LiteLLM_DailyGuardrailMetrics",
        "LiteLLM_DailyGuardrailUsageUnits",
        "LiteLLM_SpendLogGuardrailIndex",
        "LiteLLM_SpendLogToolIndex",
        "LiteLLM_ToolTable",
    ] {
        let statement = format!("SELECT count(*) FROM \"{table}\"");
        assert_eq!(
            scalar::<i64>(&client, &statement).await,
            0,
            "an empty governance seed inserts no rows into {table}"
        );
    }
    assert_eq!(
        strings(&client, USER_TEAMS_STATE, &[]).await,
        ["alice|team-blue,team-red", "bob|"],
        "an empty governance seed still publishes the membership teams"
    );
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn governance_empty_and_absent() {
    let (_server, admin_url) = postgres_server().await;
    assert_without_governance(&admin_url).await;
    assert_empty_governance(&admin_url).await;
}
