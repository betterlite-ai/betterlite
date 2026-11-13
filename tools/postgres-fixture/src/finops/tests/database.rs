use std::env::split_paths;
use std::ffi::OsString;

use clap::Parser;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, NoTls, Row};
use uuid::Uuid;

use crate::connection::{PostgresFixture, RandomDatabaseName};
use crate::error::FixtureError;
use crate::finops::FinOpsFixtureError;
use crate::seed::FixtureSeed;
use crate::tests::{FixedName, postgres_server};

use super::support::{
    ORPHAN_HASH, VANTAGE_SETTINGS, deterministic_rows, expected_page, fixture_with, sample_finops,
    sample_seed, seed_with_daily_usage,
};

// Mirrors the FOCUS export SELECT for the rows the fixture seeds.
const ORGANIZATION_JOIN: &str = "SELECT COALESCE(vt.\"organization_id\", tt.\"organization_id\"), vt.\"key_alias\", tt.\"team_alias\", org.\"organization_alias\" FROM \"LiteLLM_DailyUserSpend\" dus LEFT JOIN \"LiteLLM_VerificationToken\" vt ON dus.\"api_key\" = vt.\"token\" LEFT JOIN \"LiteLLM_TeamTable\" tt ON vt.\"team_id\" = tt.\"team_id\" LEFT JOIN \"LiteLLM_OrganizationTable\" org ON org.\"organization_id\" = COALESCE(vt.\"organization_id\", tt.\"organization_id\") WHERE dus.\"id\" = $1";

const USAGE_FIELDS: &str = "SELECT \"user_id\", \"model\", \"model_group\", \"custom_llm_provider\", \"mcp_namespaced_tool_name\", \"endpoint\", \"api_key\", \"spend\"::text, \"compression_savings_spend\"::text, \"prompt_caching_savings_spend\"::text, \"gateway_injected_caching_savings_spend\"::text, \"autorouter_savings_spend\"::text, \"cache_creation_input_tokens\", \"cache_read_input_tokens\", \"total_response_time_ms\", \"timed_requests\" FROM \"LiteLLM_DailyUserSpend\" WHERE \"id\" = $1";

#[derive(Parser)]
struct TestEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    postgres_test_url: Option<String>,
    #[arg(long, env = "PATH")]
    path: Option<OsString>,
}

fn postgres_available() -> bool {
    let environment = TestEnvironment::parse_from(["finops-fixture-tests"]);
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

async fn scalar_i64(client: &Client, statement: &str, params: &[&(dyn ToSql + Sync)]) -> i64 {
    client.query_one(statement, params).await.unwrap().get(0)
}

async fn stored_ids(client: &Client, limit: i64) -> Vec<String> {
    client
        .query(
            "SELECT \"id\" FROM \"LiteLLM_DailyUserSpend\" ORDER BY \"date\" DESC, \"created_at\" DESC LIMIT $1",
            &[&limit],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get(0))
        .collect()
}

async fn run_statement(fixture: &PostgresFixture, statement: &str, params: &[&(dyn ToSql + Sync)]) {
    let (client, driver) = connect(&fixture.options.url).await;
    client.execute(statement, params).await.unwrap();
    disconnect(client, driver).await;
}

async fn spend_of(fixture: &PostgresFixture, id: &str) -> String {
    let (client, driver) = connect(&fixture.options.url).await;
    let spend = client
        .query_one(
            "SELECT \"spend\"::text FROM \"LiteLLM_DailyUserSpend\" WHERE \"id\" = $1",
            &[&id],
        )
        .await
        .unwrap()
        .get(0);
    disconnect(client, driver).await;
    spend
}

async fn set_spend(fixture: &PostgresFixture, id: &str, spend: &str) {
    run_statement(
        fixture,
        "UPDATE \"LiteLLM_DailyUserSpend\" SET \"spend\" = $1::text::numeric::double precision WHERE \"id\" = $2",
        &[&spend, &id],
    )
    .await;
}

async fn check_sampling_boundary(admin_url: &SecretString, count: usize, limit: usize) {
    let rows = deterministic_rows(count);
    let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
        panic!("deterministic rows are not empty");
    };
    let first_updated = first.updated_at.clone();
    let last_updated = last.updated_at.clone();
    let seed = seed_with_daily_usage(rows);
    let fixture = provision(admin_url, &seed).await;
    let (client, driver) = connect(&fixture.options.url).await;
    let stored = scalar_i64(
        &client,
        "SELECT count(*) FROM \"LiteLLM_DailyUserSpend\"",
        &[],
    )
    .await;
    assert_eq!(stored, i64::try_from(count).unwrap());
    let page = stored_ids(&client, i64::try_from(limit).unwrap()).await;
    assert_eq!(page, expected_page(count, limit));
    let all = stored_ids(&client, i64::try_from(count).unwrap()).await;
    assert_eq!(all.len(), count);
    check_updated_bounds(&client, &first_updated, &last_updated, count).await;
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

async fn check_updated_bounds(client: &Client, first: &str, last: &str, count: usize) {
    let inclusive = scalar_i64(
        client,
        "SELECT count(*) FROM \"LiteLLM_DailyUserSpend\" WHERE \"updated_at\" >= $1::text::timestamp(3) AND \"updated_at\" <= $2::text::timestamp(3)",
        &[&first, &last],
    )
    .await;
    assert_eq!(inclusive, i64::try_from(count).unwrap());
    let exclusive = scalar_i64(
        client,
        "SELECT count(*) FROM \"LiteLLM_DailyUserSpend\" WHERE \"updated_at\" > $1::text::timestamp(3) AND \"updated_at\" < $2::text::timestamp(3)",
        &[&first, &last],
    )
    .await;
    assert_eq!(
        exclusive,
        i64::try_from(count.checked_sub(2).unwrap()).unwrap()
    );
}

struct ExpectedJoin {
    id: &'static str,
    organization: Option<&'static str>,
    key_alias: Option<&'static str>,
    team_alias: Option<&'static str>,
    organization_alias: Option<&'static str>,
}

const EXPECTED_JOINS: [ExpectedJoin; 3] = [
    ExpectedJoin {
        id: "row-key",
        organization: Some("org-key"),
        key_alias: Some("alice-key"),
        team_alias: Some("Red Team"),
        organization_alias: Some("Key Org"),
    },
    ExpectedJoin {
        id: "row-team",
        organization: Some("org-other"),
        key_alias: None,
        team_alias: Some("Blue Team"),
        organization_alias: Some("Other Org"),
    },
    ExpectedJoin {
        id: "row-orphan",
        organization: None,
        key_alias: None,
        team_alias: None,
        organization_alias: None,
    },
];

async fn check_organization_precedence(admin_url: &SecretString) {
    let fixture = provision(admin_url, &sample_seed()).await;
    let (client, driver) = connect(&fixture.options.url).await;
    assert_organization_joins(&client).await;
    assert_key_identity(&client).await;
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

async fn assert_organization_joins(client: &Client) {
    for expected in &EXPECTED_JOINS {
        let row = client
            .query_one(ORGANIZATION_JOIN, &[&expected.id])
            .await
            .unwrap();
        assert_eq!(
            row.get::<_, Option<String>>(0).as_deref(),
            expected.organization
        );
        assert_eq!(
            row.get::<_, Option<String>>(1).as_deref(),
            expected.key_alias
        );
        assert_eq!(
            row.get::<_, Option<String>>(2).as_deref(),
            expected.team_alias
        );
        assert_eq!(
            row.get::<_, Option<String>>(3).as_deref(),
            expected.organization_alias
        );
    }
}

async fn assert_key_identity(client: &Client) {
    let token = FixtureSeed::key_hash("sk-alice-token");
    let key = client
        .query_one(
            "SELECT \"user_id\", \"team_id\" FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = $1",
            &[&token],
        )
        .await
        .unwrap();
    assert_eq!(key.get::<_, Option<String>>(0).as_deref(), Some("alice"));
    assert_eq!(key.get::<_, Option<String>>(1).as_deref(), Some("team-red"));
    let role: Option<String> = client
        .query_one(
            "SELECT \"user_role\" FROM \"LiteLLM_UserTable\" WHERE \"user_id\" = 'alice'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(role.as_deref(), Some("proxy_admin"));
}

async fn check_null_models_and_decimals(admin_url: &SecretString) {
    let fixture = provision(admin_url, &sample_seed()).await;
    let (client, driver) = connect(&fixture.options.url).await;
    let row = client
        .query_one(USAGE_FIELDS, &[&"row-orphan"])
        .await
        .unwrap();
    assert_null_fields(&row);
    assert_money_text(&row);
    assert_token_counters(&row);
    let keyed = client.query_one(USAGE_FIELDS, &[&"row-key"]).await.unwrap();
    assert_eq!(
        keyed.get::<_, String>(6),
        FixtureSeed::key_hash("sk-alice-token")
    );
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

fn assert_null_fields(row: &Row) {
    for column in 0..6 {
        assert_eq!(row.get::<_, Option<String>>(column), None);
    }
}

fn assert_money_text(row: &Row) {
    assert_eq!(row.get::<_, String>(6), ORPHAN_HASH);
    assert_eq!(row.get::<_, String>(7), "1.25");
    assert_eq!(row.get::<_, String>(8), "0.5");
    assert_eq!(row.get::<_, String>(9), "0.25");
    assert_eq!(row.get::<_, String>(10), "0.125");
    assert_eq!(row.get::<_, String>(11), "0.0625");
}

fn assert_token_counters(row: &Row) {
    assert_eq!(row.get::<_, i64>(12), 4);
    assert_eq!(row.get::<_, i64>(13), 3);
    assert_eq!(row.get::<_, i64>(14), 120);
    assert_eq!(row.get::<_, i64>(15), 2);
}

async fn check_serialized_config(admin_url: &SecretString) {
    let fixture = provision(admin_url, &sample_seed()).await;
    let (client, driver) = connect(&fixture.options.url).await;
    let mut rows = client
        .query(
            "SELECT \"param_name\", jsonb_typeof(\"param_value\"), \"param_value\" #>> '{}', (\"param_value\" IS NULL) FROM \"LiteLLM_Config\" ORDER BY \"param_name\"",
            &[],
        )
        .await
        .unwrap()
        .into_iter();
    let cloudzero = rows.next().expect("the cloudzero row is seeded");
    assert_eq!(cloudzero.get::<_, String>(0), "cloudzero_settings");
    assert_eq!(cloudzero.get::<_, Option<String>>(1), None);
    assert_eq!(cloudzero.get::<_, Option<String>>(2), None);
    assert!(cloudzero.get::<_, bool>(3));
    let vantage = rows.next().expect("the vantage row is seeded");
    assert_eq!(vantage.get::<_, String>(0), "vantage_settings");
    assert_eq!(
        vantage.get::<_, Option<String>>(1).as_deref(),
        Some("string")
    );
    assert_eq!(
        vantage.get::<_, Option<String>>(2).as_deref(),
        Some(VANTAGE_SETTINGS)
    );
    assert!(!vantage.get::<_, bool>(3));
    assert!(rows.next().is_none());
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

async fn check_independent_databases(admin_url: &SecretString) {
    let seed = sample_seed();
    let first = provision(admin_url, &seed).await;
    let second = provision(admin_url, &seed).await;
    assert_untouched(&first, &second).await;
    set_spend(&first, "row-key", "99.5").await;
    assert_mutated(&first, &second).await;
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
}

async fn assert_untouched(first: &PostgresFixture, second: &PostgresFixture) {
    assert_eq!(spend_of(first, "row-key").await, "1.25");
    assert_eq!(spend_of(second, "row-key").await, "1.25");
}

async fn assert_mutated(first: &PostgresFixture, second: &PostgresFixture) {
    assert_eq!(spend_of(first, "row-key").await, "99.5");
    assert_eq!(spend_of(second, "row-key").await, "1.25");
    assert_eq!(spend_of(second, "row-orphan").await, "1.25");
}

async fn check_failed_insert_cleanup(admin_url: &SecretString) {
    let mut finops = sample_finops();
    let Some(organization) = finops.organizations.first_mut() else {
        panic!("the sample seed has an organization");
    };
    organization.organization_alias = "bad\0alias".to_owned();
    let seed = fixture_with(finops);
    let name = format!("finops_failed_{}", Uuid::new_v4().simple());
    let result = PostgresFixture::provision_with_name_generator(
        Some(admin_url),
        &seed,
        &mut FixedName(name.clone()),
    )
    .resolve()
    .await;
    assert!(matches!(
        result,
        Err(FixtureError::FinOps(FinOpsFixtureError::Database(_)))
    ));
    let (admin, driver) = connect(admin_url).await;
    let exists = admin
        .query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&name])
        .await
        .unwrap()
        .is_some();
    assert!(!exists, "a failed seed leaves no fixture database behind");
    disconnect(admin, driver).await;
}

#[derive(Clone, Copy, Debug)]
enum DatabaseCase {
    FiftyOneRows,
    FiveHundredOneRows,
    OrganizationPrecedence,
    NullModelsAndDecimals,
    SerializedConfig,
    IndependentDatabases,
    FailedInsertCleanup,
}

#[rstest]
#[case::fifty_one_rows(DatabaseCase::FiftyOneRows)]
#[case::five_hundred_one_rows(DatabaseCase::FiveHundredOneRows)]
#[case::organization_precedence(DatabaseCase::OrganizationPrecedence)]
#[case::null_models_and_decimals(DatabaseCase::NullModelsAndDecimals)]
#[case::serialized_config(DatabaseCase::SerializedConfig)]
#[case::independent_databases(DatabaseCase::IndependentDatabases)]
#[case::failed_insert_cleanup(DatabaseCase::FailedInsertCleanup)]
#[tokio::test]
async fn finops_seed_database_contract(#[case] case: DatabaseCase) {
    if !postgres_available() {
        return;
    }
    let (_server, admin_url) = postgres_server().await;
    match case {
        DatabaseCase::FiftyOneRows => check_sampling_boundary(&admin_url, 51, 50).await,
        DatabaseCase::FiveHundredOneRows => check_sampling_boundary(&admin_url, 501, 500).await,
        DatabaseCase::OrganizationPrecedence => check_organization_precedence(&admin_url).await,
        DatabaseCase::NullModelsAndDecimals => {
            check_null_models_and_decimals(&admin_url).await;
        }
        DatabaseCase::SerializedConfig => check_serialized_config(&admin_url).await,
        DatabaseCase::IndependentDatabases => check_independent_databases(&admin_url).await,
        DatabaseCase::FailedInsertCleanup => check_failed_insert_cleanup(&admin_url).await,
    }
}
