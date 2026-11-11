#![cfg(test)]

use std::cmp::Reverse;
use std::env::split_paths;
use std::ffi::OsString;

use clap::Parser;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, NoTls};
use uuid::Uuid;

use crate::connection::{PostgresFixture, RandomDatabaseName};
use crate::error::FixtureError;
use crate::finops::FinOpsFixtureError;
use crate::finops::model::{
    ConfigName, ConfigRow, FinOpsSeed, FinOpsUsageRow, KeyMetadataRow, OrganizationRow, UsageApiKey,
};
use crate::seed::{BudgetRow, CallerRole, FixtureSeed, KeyRow, TeamRow, UserRow};
use crate::tests::{FixedName, postgres_server};

const LIMIT: usize = 1024;
const OVER_LIMIT: usize = 1025;
const ORPHAN_HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const VANTAGE_SETTINGS: &str = "{\"api_key\":\"ciphertext-not-a-key\"}";
const DATES: [&str; 3] = ["2025-01-01", "2025-01-02", "2025-01-03"];

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

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn fixture_with(finops: FinOpsSeed) -> FixtureSeed {
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
                team_id: Some("team-red".to_owned()),
                role: Some(CallerRole::ProxyAdmin),
            },
            KeyRow {
                token: secret("sk-second-red"),
                user_id: "alice".to_owned(),
                team_id: Some("team-red".to_owned()),
                role: None,
            },
            KeyRow {
                token: secret("sk-bob-token"),
                user_id: "bob".to_owned(),
                team_id: Some("team-blue".to_owned()),
                role: None,
            },
            KeyRow {
                token: secret("sk-teamless-token"),
                user_id: "bob".to_owned(),
                team_id: None,
                role: None,
            },
        ],
        teams: vec![
            TeamRow {
                team_id: "team-red".to_owned(),
                team_alias: Some("Red Team".to_owned()),
            },
            TeamRow {
                team_id: "team-blue".to_owned(),
                team_alias: Some("Blue Team".to_owned()),
            },
        ],
        memberships: Vec::new(),
        budgets: vec![BudgetRow {
            budget_id: "budget-1".to_owned(),
            user_id: "alice".to_owned(),
            max_budget: Some(Decimal::new(1000, 2)),
        }],
        jwt_mappings: Vec::new(),
        daily_usage: None,
        finops: Some(finops),
        discovery_skills: None,
        search_tools: None,
        dashboard: None,
        dashboard_governance: None,
    }
}

fn empty_finops() -> FinOpsSeed {
    FinOpsSeed {
        organizations: Vec::new(),
        key_metadata: Vec::new(),
        daily_usage: Vec::new(),
        config_rows: Vec::new(),
    }
}

fn organization(id: &str, alias: &str) -> OrganizationRow {
    OrganizationRow {
        organization_id: id.to_owned(),
        organization_alias: alias.to_owned(),
        budget_id: "budget-1".to_owned(),
        created_by: "alice".to_owned(),
        updated_by: "alice".to_owned(),
        models: Vec::new(),
    }
}

fn usage_row(id: &str, api_key: UsageApiKey) -> FinOpsUsageRow {
    FinOpsUsageRow {
        id: id.to_owned(),
        user_id: Some("alice".to_owned()),
        date: "2025-01-01".to_owned(),
        api_key,
        model: Some("openai/gpt-5".to_owned()),
        model_group: Some("default".to_owned()),
        custom_llm_provider: Some("openai".to_owned()),
        mcp_namespaced_tool_name: Some("mcp__tool".to_owned()),
        endpoint: Some("/v1/chat/completions".to_owned()),
        prompt_tokens: 10,
        completion_tokens: 5,
        cache_read_input_tokens: 3,
        cache_creation_input_tokens: 4,
        compression_saved_tokens: 2,
        compression_savings_spend: Decimal::new(5, 1),
        prompt_caching_savings_spend: Decimal::new(25, 2),
        gateway_injected_savings: Decimal::new(125, 3),
        autorouter_savings_spend: Decimal::new(625, 4),
        spend: Decimal::new(125, 2),
        api_requests: 2,
        successful_requests: 1,
        failed_requests: 1,
        total_response_time_ms: 120,
        timed_requests: 2,
        created_at: "2025-01-01T00:00:00".to_owned(),
        updated_at: "2025-01-01T00:00:05".to_owned(),
    }
}

fn sample_finops() -> FinOpsSeed {
    let mut orphan = usage_row(
        "row-orphan",
        UsageApiKey::OrphanHash(ORPHAN_HASH.to_owned()),
    );
    orphan.user_id = None;
    orphan.model = None;
    orphan.model_group = None;
    orphan.custom_llm_provider = None;
    orphan.mcp_namespaced_tool_name = None;
    orphan.endpoint = None;
    FinOpsSeed {
        organizations: vec![
            organization("org-key", "Key Org"),
            organization("org-team", "Team Org"),
            organization("org-other", "Other Org"),
        ],
        key_metadata: vec![
            KeyMetadataRow {
                token: secret("sk-alice-token"),
                key_alias: Some("alice-key".to_owned()),
                organization_id: Some("org-key".to_owned()),
                team_organization_id: Some("org-team".to_owned()),
            },
            KeyMetadataRow {
                token: secret("sk-bob-token"),
                key_alias: None,
                organization_id: None,
                team_organization_id: Some("org-other".to_owned()),
            },
        ],
        daily_usage: vec![
            usage_row("row-key", UsageApiKey::Key(secret("sk-alice-token"))),
            usage_row("row-team", UsageApiKey::Key(secret("sk-bob-token"))),
            orphan,
        ],
        config_rows: vec![
            ConfigRow {
                name: ConfigName::VantageSettings,
                serialized_value: Some(secret(VANTAGE_SETTINGS)),
            },
            ConfigRow {
                name: ConfigName::CloudzeroSettings,
                serialized_value: None,
            },
        ],
    }
}

fn sample_seed() -> FixtureSeed {
    fixture_with(sample_finops())
}

fn deterministic_rows(count: usize) -> Vec<FinOpsUsageRow> {
    (0..count)
        .map(|index| {
            let date = DATES
                .get(index.checked_rem(3).unwrap_or(0))
                .expect("a date cycles");
            let timestamp = format!("2025-01-01T00:00:00.{index:03}");
            FinOpsUsageRow {
                id: format!("row-{index:04}"),
                user_id: Some("alice".to_owned()),
                date: (*date).to_owned(),
                api_key: UsageApiKey::Key(secret("sk-alice-token")),
                model: Some("openai/gpt-5".to_owned()),
                model_group: Some("default".to_owned()),
                custom_llm_provider: Some("openai".to_owned()),
                mcp_namespaced_tool_name: None,
                endpoint: Some(format!("/v1/chat/{index}")),
                prompt_tokens: 1,
                completion_tokens: 1,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
                compression_saved_tokens: 0,
                compression_savings_spend: Decimal::ZERO,
                prompt_caching_savings_spend: Decimal::ZERO,
                gateway_injected_savings: Decimal::ZERO,
                autorouter_savings_spend: Decimal::ZERO,
                spend: Decimal::ZERO,
                api_requests: 1,
                successful_requests: 1,
                failed_requests: 0,
                total_response_time_ms: 10,
                timed_requests: 1,
                created_at: timestamp.clone(),
                updated_at: timestamp,
            }
        })
        .collect()
}

fn expected_page(count: usize, limit: usize) -> Vec<String> {
    let mut indexes = (0..count).collect::<Vec<_>>();
    indexes.sort_by_key(|index| (Reverse(index.checked_rem(3).unwrap_or(0)), Reverse(*index)));
    indexes.truncate(limit);
    indexes
        .into_iter()
        .map(|index| format!("row-{index:04}"))
        .collect()
}

fn seed_with_daily_usage(rows: Vec<FinOpsUsageRow>) -> FixtureSeed {
    let mut finops = empty_finops();
    finops.daily_usage = rows;
    fixture_with(finops)
}

fn seed_with_organizations(organizations: Vec<OrganizationRow>) -> FixtureSeed {
    let mut finops = empty_finops();
    finops.organizations = organizations;
    fixture_with(finops)
}

fn seed_with_key_metadata(key_metadata: Vec<KeyMetadataRow>) -> FixtureSeed {
    let mut finops = sample_finops();
    finops.key_metadata = key_metadata;
    fixture_with(finops)
}

fn seed_with_config_rows(config_rows: Vec<ConfigRow>) -> FixtureSeed {
    let mut finops = empty_finops();
    finops.config_rows = config_rows;
    fixture_with(finops)
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
    let (client, driver) = connect(&fixture.options.url).await;
    client
        .execute(
            "UPDATE \"LiteLLM_DailyUserSpend\" SET \"spend\" = $1::numeric::double precision WHERE \"id\" = $2",
            &[&spend, &id],
        )
        .await
        .unwrap();
    disconnect(client, driver).await;
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
    let inclusive = scalar_i64(
        &client,
        "SELECT count(*) FROM \"LiteLLM_DailyUserSpend\" WHERE \"updated_at\" >= $1::timestamp(3) AND \"updated_at\" <= $2::timestamp(3)",
        &[&first_updated, &last_updated],
    )
    .await;
    assert_eq!(inclusive, i64::try_from(count).unwrap());
    let exclusive = scalar_i64(
        &client,
        "SELECT count(*) FROM \"LiteLLM_DailyUserSpend\" WHERE \"updated_at\" > $1::timestamp(3) AND \"updated_at\" < $2::timestamp(3)",
        &[&first_updated, &last_updated],
    )
    .await;
    assert_eq!(
        exclusive,
        i64::try_from(count.checked_sub(2).unwrap()).unwrap()
    );
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

async fn check_organization_precedence(admin_url: &SecretString) {
    let fixture = provision(admin_url, &sample_seed()).await;
    let (client, driver) = connect(&fixture.options.url).await;
    let statement = "SELECT COALESCE(vt.\"organization_id\", tt.\"organization_id\"), vt.\"key_alias\", tt.\"team_alias\", org.\"organization_alias\" FROM \"LiteLLM_DailyUserSpend\" dus LEFT JOIN \"LiteLLM_VerificationToken\" vt ON dus.\"api_key\" = vt.\"token\" LEFT JOIN \"LiteLLM_TeamTable\" tt ON vt.\"team_id\" = tt.\"team_id\" LEFT JOIN \"LiteLLM_OrganizationTable\" org ON org.\"organization_id\" = COALESCE(vt.\"organization_id\", tt.\"organization_id\") WHERE dus.\"id\" = $1";
    let key_row = client.query_one(statement, &[&"row-key"]).await.unwrap();
    assert_eq!(
        key_row.get::<_, Option<String>>(0).as_deref(),
        Some("org-key")
    );
    assert_eq!(
        key_row.get::<_, Option<String>>(1).as_deref(),
        Some("alice-key")
    );
    assert_eq!(
        key_row.get::<_, Option<String>>(2).as_deref(),
        Some("Red Team")
    );
    assert_eq!(
        key_row.get::<_, Option<String>>(3).as_deref(),
        Some("Key Org")
    );
    let team_row = client.query_one(statement, &[&"row-team"]).await.unwrap();
    assert_eq!(
        team_row.get::<_, Option<String>>(0).as_deref(),
        Some("org-other")
    );
    assert_eq!(
        team_row.get::<_, Option<String>>(2).as_deref(),
        Some("Blue Team")
    );
    assert_eq!(
        team_row.get::<_, Option<String>>(3).as_deref(),
        Some("Other Org")
    );
    let orphan_row = client.query_one(statement, &[&"row-orphan"]).await.unwrap();
    assert_eq!(orphan_row.get::<_, Option<String>>(0), None);
    assert_eq!(orphan_row.get::<_, Option<String>>(1), None);
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
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
}

async fn check_null_models_and_decimals(admin_url: &SecretString) {
    let fixture = provision(admin_url, &sample_seed()).await;
    let (client, driver) = connect(&fixture.options.url).await;
    let statement = "SELECT \"user_id\", \"model\", \"model_group\", \"custom_llm_provider\", \"mcp_namespaced_tool_name\", \"endpoint\", \"api_key\", \"spend\"::text, \"compression_savings_spend\"::text, \"prompt_caching_savings_spend\"::text, \"gateway_injected_caching_savings_spend\"::text, \"autorouter_savings_spend\"::text, \"cache_creation_input_tokens\", \"cache_read_input_tokens\", \"total_response_time_ms\", \"timed_requests\" FROM \"LiteLLM_DailyUserSpend\" WHERE \"id\" = $1";
    let row = client.query_one(statement, &[&"row-orphan"]).await.unwrap();
    for column in 0..6 {
        assert_eq!(row.get::<_, Option<String>>(column), None);
    }
    assert_eq!(row.get::<_, String>(6), ORPHAN_HASH);
    assert_eq!(row.get::<_, String>(7), "1.25");
    assert_eq!(row.get::<_, String>(8), "0.5");
    assert_eq!(row.get::<_, String>(9), "0.25");
    assert_eq!(row.get::<_, String>(10), "0.125");
    assert_eq!(row.get::<_, String>(11), "0.0625");
    assert_eq!(row.get::<_, i64>(12), 4);
    assert_eq!(row.get::<_, i64>(13), 3);
    assert_eq!(row.get::<_, i64>(14), 120);
    assert_eq!(row.get::<_, i64>(15), 2);
    let keyed = client
        .query_one(
            "SELECT \"api_key\" FROM \"LiteLLM_DailyUserSpend\" WHERE \"id\" = $1",
            &[&"row-key"],
        )
        .await
        .unwrap();
    assert_eq!(
        keyed.get::<_, String>(0),
        FixtureSeed::key_hash("sk-alice-token")
    );
    disconnect(client, driver).await;
    fixture.close_async().await.unwrap();
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
    assert_eq!(spend_of(&first, "row-key").await, "1.25");
    assert_eq!(spend_of(&second, "row-key").await, "1.25");
    set_spend(&first, "row-key", "99.5").await;
    assert_eq!(spend_of(&first, "row-key").await, "99.5");
    assert_eq!(spend_of(&second, "row-key").await, "1.25");
    assert_eq!(spend_of(&second, "row-orphan").await, "1.25");
    first.close_async().await.unwrap();
    second.close_async().await.unwrap();
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedError {
    Accepted,
    RedactedDiagnostics,
    TooManyRows {
        list: &'static str,
        count: usize,
        limit: usize,
    },
    DuplicateOrganization,
    MissingBudget,
    MissingUser(&'static str),
    UnknownKeyMetadata,
    DuplicateKeyMetadata,
    MissingOrganization(&'static str),
    KeyMetadataWithoutTeam,
    MissingTeam,
    ConflictingTeamOrganization,
    DuplicateDailyUsageId,
    MissingUsageUser,
    UnknownUsageKey,
    InvalidOrphanHash,
    InvalidTimestamp(&'static str),
    DuplicateUsageTuple,
    DuplicateConfigName,
}

fn error_matches(source: &FinOpsFixtureError, expected: ExpectedError) -> bool {
    match (source, expected) {
        (
            FinOpsFixtureError::TooManyRows { list, count, limit },
            ExpectedError::TooManyRows {
                list: wanted_list,
                count: wanted_count,
                limit: wanted_limit,
            },
        ) => *list == wanted_list && *count == wanted_count && *limit == wanted_limit,
        (
            FinOpsFixtureError::DuplicateOrganization { .. },
            ExpectedError::DuplicateOrganization,
        )
        | (FinOpsFixtureError::MissingBudget { .. }, ExpectedError::MissingBudget)
        | (FinOpsFixtureError::UnknownKeyMetadata { .. }, ExpectedError::UnknownKeyMetadata)
        | (FinOpsFixtureError::DuplicateKeyMetadata { .. }, ExpectedError::DuplicateKeyMetadata)
        | (
            FinOpsFixtureError::KeyMetadataWithoutTeam { .. },
            ExpectedError::KeyMetadataWithoutTeam,
        )
        | (FinOpsFixtureError::MissingTeam { .. }, ExpectedError::MissingTeam)
        | (
            FinOpsFixtureError::ConflictingTeamOrganization { .. },
            ExpectedError::ConflictingTeamOrganization,
        )
        | (
            FinOpsFixtureError::DuplicateDailyUsageId { .. },
            ExpectedError::DuplicateDailyUsageId,
        )
        | (FinOpsFixtureError::MissingUsageUser { .. }, ExpectedError::MissingUsageUser)
        | (FinOpsFixtureError::UnknownUsageKey { .. }, ExpectedError::UnknownUsageKey)
        | (FinOpsFixtureError::InvalidOrphanHash { .. }, ExpectedError::InvalidOrphanHash)
        | (FinOpsFixtureError::DuplicateUsageTuple { .. }, ExpectedError::DuplicateUsageTuple)
        | (FinOpsFixtureError::DuplicateConfigName { .. }, ExpectedError::DuplicateConfigName) => {
            true
        }
        (FinOpsFixtureError::MissingUser { field, .. }, ExpectedError::MissingUser(wanted)) => {
            *field == wanted
        }
        (
            FinOpsFixtureError::MissingOrganization { field, .. },
            ExpectedError::MissingOrganization(wanted),
        ) => *field == wanted,
        (
            FinOpsFixtureError::InvalidTimestamp { field, .. },
            ExpectedError::InvalidTimestamp(wanted),
        ) => *field == wanted,
        _ => false,
    }
}

fn empty_usage_seed() -> FixtureSeed {
    seed_with_daily_usage(Vec::new())
}

fn duplicate_organization_seed() -> FixtureSeed {
    seed_with_organizations(vec![
        organization("org-key", "Key Org"),
        organization("org-key", "Key Org Again"),
    ])
}

fn missing_budget_seed() -> FixtureSeed {
    let mut organization = organization("org-key", "Key Org");
    organization.budget_id = "budget-missing".to_owned();
    seed_with_organizations(vec![organization])
}

fn missing_creator_seed() -> FixtureSeed {
    let mut organization = organization("org-key", "Key Org");
    organization.created_by = "carol".to_owned();
    seed_with_organizations(vec![organization])
}

fn missing_updater_seed() -> FixtureSeed {
    let mut organization = organization("org-key", "Key Org");
    organization.updated_by = "carol".to_owned();
    seed_with_organizations(vec![organization])
}

fn unknown_metadata_key_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![KeyMetadataRow {
        token: secret("sk-missing"),
        key_alias: None,
        organization_id: None,
        team_organization_id: None,
    }])
}

fn duplicate_metadata_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![
        KeyMetadataRow {
            token: secret("sk-alice-token"),
            key_alias: None,
            organization_id: None,
            team_organization_id: None,
        },
        KeyMetadataRow {
            token: secret("sk-alice-token"),
            key_alias: Some("alice-key".to_owned()),
            organization_id: None,
            team_organization_id: None,
        },
    ])
}

fn missing_metadata_organization_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![KeyMetadataRow {
        token: secret("sk-alice-token"),
        key_alias: None,
        organization_id: Some("org-missing".to_owned()),
        team_organization_id: None,
    }])
}

fn missing_team_organization_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![KeyMetadataRow {
        token: secret("sk-alice-token"),
        key_alias: None,
        organization_id: None,
        team_organization_id: Some("org-missing".to_owned()),
    }])
}

fn key_without_team_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![KeyMetadataRow {
        token: secret("sk-teamless-token"),
        key_alias: None,
        organization_id: None,
        team_organization_id: Some("org-team".to_owned()),
    }])
}

fn missing_team_seed() -> FixtureSeed {
    let mut fixture = seed_with_key_metadata(vec![KeyMetadataRow {
        token: secret("sk-alice-token"),
        key_alias: None,
        organization_id: None,
        team_organization_id: Some("org-team".to_owned()),
    }]);
    let Some(key) = fixture.keys.first_mut() else {
        panic!("the fixture has a key");
    };
    key.team_id = Some("team-missing".to_owned());
    fixture
}

fn conflicting_team_seed() -> FixtureSeed {
    seed_with_key_metadata(vec![
        KeyMetadataRow {
            token: secret("sk-alice-token"),
            key_alias: None,
            organization_id: None,
            team_organization_id: Some("org-team".to_owned()),
        },
        KeyMetadataRow {
            token: secret("sk-second-red"),
            key_alias: None,
            organization_id: None,
            team_organization_id: Some("org-other".to_owned()),
        },
    ])
}

fn duplicate_usage_id_seed() -> FixtureSeed {
    seed_with_daily_usage(vec![
        usage_row("row-same", UsageApiKey::Key(secret("sk-alice-token"))),
        usage_row("row-same", UsageApiKey::Key(secret("sk-bob-token"))),
    ])
}

fn missing_usage_user_seed() -> FixtureSeed {
    let mut row = usage_row("row-user", UsageApiKey::Key(secret("sk-alice-token")));
    row.user_id = Some("carol".to_owned());
    seed_with_daily_usage(vec![row])
}

fn unknown_usage_key_seed() -> FixtureSeed {
    seed_with_daily_usage(vec![usage_row(
        "row-key",
        UsageApiKey::Key(secret("sk-missing")),
    )])
}

fn invalid_orphan_hash_seed() -> FixtureSeed {
    seed_with_daily_usage(vec![usage_row(
        "row-orphan",
        UsageApiKey::OrphanHash("abc".to_owned()),
    )])
}

fn invalid_created_at_seed() -> FixtureSeed {
    let mut row = usage_row("row-time", UsageApiKey::Key(secret("sk-alice-token")));
    row.created_at = "not-a-timestamp".to_owned();
    seed_with_daily_usage(vec![row])
}

fn invalid_updated_at_seed() -> FixtureSeed {
    let mut row = usage_row("row-time", UsageApiKey::Key(secret("sk-alice-token")));
    row.updated_at = "2025-13-01T00:00:00".to_owned();
    seed_with_daily_usage(vec![row])
}

fn duplicate_tuple_seed() -> FixtureSeed {
    let first = usage_row("row-first", UsageApiKey::Key(secret("sk-alice-token")));
    let mut second = first.clone();
    second.id = "row-second".to_owned();
    seed_with_daily_usage(vec![first, second])
}

fn null_tuple_seed() -> FixtureSeed {
    let mut first = usage_row("row-first", UsageApiKey::Key(secret("sk-alice-token")));
    let mut second = usage_row("row-second", UsageApiKey::Key(secret("sk-alice-token")));
    first.model = None;
    second.model = None;
    seed_with_daily_usage(vec![first, second])
}

fn duplicate_config_name_seed() -> FixtureSeed {
    seed_with_config_rows(vec![
        ConfigRow {
            name: ConfigName::VantageSettings,
            serialized_value: None,
        },
        ConfigRow {
            name: ConfigName::VantageSettings,
            serialized_value: Some(secret("second")),
        },
    ])
}

fn repeated_organizations(count: usize) -> Vec<OrganizationRow> {
    (0..count)
        .map(|index| organization(&format!("org-{index}"), "Org"))
        .collect()
}

fn repeated_metadata_tokens(count: usize) -> Vec<KeyMetadataRow> {
    (0..count)
        .map(|_| KeyMetadataRow {
            token: secret("sk-alice-token"),
            key_alias: None,
            organization_id: None,
            team_organization_id: None,
        })
        .collect()
}

fn too_many_organizations_seed() -> FixtureSeed {
    seed_with_organizations(repeated_organizations(OVER_LIMIT))
}

fn too_many_key_metadata_seed() -> FixtureSeed {
    seed_with_key_metadata(repeated_metadata_tokens(OVER_LIMIT))
}

fn too_many_daily_usage_seed() -> FixtureSeed {
    seed_with_daily_usage(deterministic_rows(OVER_LIMIT))
}

fn too_many_config_rows_seed() -> FixtureSeed {
    seed_with_config_rows(vec![
        ConfigRow {
            name: ConfigName::VantageSettings,
            serialized_value: None,
        };
        OVER_LIMIT
    ])
}

fn boundary_organizations_seed() -> FixtureSeed {
    seed_with_organizations(repeated_organizations(LIMIT))
}

fn boundary_key_metadata_seed() -> FixtureSeed {
    let mut fixture = fixture_with(empty_finops());
    fixture.keys = (0..LIMIT)
        .map(|index| KeyRow {
            token: secret(&format!("sk-key-{index}")),
            user_id: "alice".to_owned(),
            team_id: None,
            role: None,
        })
        .collect();
    let mut finops = empty_finops();
    finops.key_metadata = (0..LIMIT)
        .map(|index| KeyMetadataRow {
            token: secret(&format!("sk-key-{index}")),
            key_alias: None,
            organization_id: None,
            team_organization_id: None,
        })
        .collect();
    fixture.finops = Some(finops);
    fixture
}

fn boundary_daily_usage_seed() -> FixtureSeed {
    let rows = (0..LIMIT)
        .map(|index| {
            let mut row = usage_row(
                &format!("row-{index}"),
                UsageApiKey::Key(secret("sk-alice-token")),
            );
            row.endpoint = Some(format!("/v1/{index}"));
            row
        })
        .collect();
    seed_with_daily_usage(rows)
}

#[rstest]
#[case::sample(sample_seed, ExpectedError::Accepted)]
#[case::empty_usage(empty_usage_seed, ExpectedError::Accepted)]
#[case::redacted_diagnostics(sample_seed, ExpectedError::RedactedDiagnostics)]
#[case::null_tuple(null_tuple_seed, ExpectedError::Accepted)]
#[case::too_many_organizations(
    too_many_organizations_seed,
    ExpectedError::TooManyRows { list: "organizations", count: OVER_LIMIT, limit: LIMIT }
)]
#[case::too_many_key_metadata(
    too_many_key_metadata_seed,
    ExpectedError::TooManyRows { list: "key_metadata", count: OVER_LIMIT, limit: LIMIT }
)]
#[case::too_many_daily_usage(
    too_many_daily_usage_seed,
    ExpectedError::TooManyRows { list: "daily_usage", count: OVER_LIMIT, limit: LIMIT }
)]
#[case::too_many_config_rows(
    too_many_config_rows_seed,
    ExpectedError::TooManyRows { list: "config_rows", count: OVER_LIMIT, limit: LIMIT }
)]
#[case::boundary_organizations(boundary_organizations_seed, ExpectedError::Accepted)]
#[case::boundary_key_metadata(boundary_key_metadata_seed, ExpectedError::Accepted)]
#[case::boundary_daily_usage(boundary_daily_usage_seed, ExpectedError::Accepted)]
#[case::duplicate_organization(duplicate_organization_seed, ExpectedError::DuplicateOrganization)]
#[case::missing_budget(missing_budget_seed, ExpectedError::MissingBudget)]
#[case::missing_creator(missing_creator_seed, ExpectedError::MissingUser("created_by"))]
#[case::missing_updater(missing_updater_seed, ExpectedError::MissingUser("updated_by"))]
#[case::unknown_metadata_key(unknown_metadata_key_seed, ExpectedError::UnknownKeyMetadata)]
#[case::duplicate_metadata(duplicate_metadata_seed, ExpectedError::DuplicateKeyMetadata)]
#[case::missing_metadata_organization(
    missing_metadata_organization_seed,
    ExpectedError::MissingOrganization("organization_id")
)]
#[case::missing_team_organization(
    missing_team_organization_seed,
    ExpectedError::MissingOrganization("team_organization_id")
)]
#[case::key_without_team(key_without_team_seed, ExpectedError::KeyMetadataWithoutTeam)]
#[case::missing_team(missing_team_seed, ExpectedError::MissingTeam)]
#[case::conflicting_team(conflicting_team_seed, ExpectedError::ConflictingTeamOrganization)]
#[case::duplicate_usage_id(duplicate_usage_id_seed, ExpectedError::DuplicateDailyUsageId)]
#[case::missing_usage_user(missing_usage_user_seed, ExpectedError::MissingUsageUser)]
#[case::unknown_usage_key(unknown_usage_key_seed, ExpectedError::UnknownUsageKey)]
#[case::invalid_orphan_hash(invalid_orphan_hash_seed, ExpectedError::InvalidOrphanHash)]
#[case::invalid_created_at(invalid_created_at_seed, ExpectedError::InvalidTimestamp("created_at"))]
#[case::invalid_updated_at(invalid_updated_at_seed, ExpectedError::InvalidTimestamp("updated_at"))]
#[case::duplicate_tuple(duplicate_tuple_seed, ExpectedError::DuplicateUsageTuple)]
#[case::duplicate_config_name(duplicate_config_name_seed, ExpectedError::DuplicateConfigName)]
fn finops_seed_validation_contract(
    #[case] build: fn() -> FixtureSeed,
    #[case] expected: ExpectedError,
) {
    let fixture = build();
    if expected == ExpectedError::RedactedDiagnostics {
        let finops = fixture.finops.as_ref().expect("the seed carries finops");
        let debug = format!("{finops:?}");
        assert!(!debug.contains("sk-alice-token"));
        assert!(!debug.contains(VANTAGE_SETTINGS));
        assert!(debug.contains("[REDACTED]"));
        let rejected = unknown_metadata_key_seed();
        let error = rejected.validate().expect_err("the seed is rejected");
        assert!(!format!("{error}").contains("sk-missing"));
        assert!(!format!("{error:?}").contains("sk-missing"));
        return;
    }
    if expected == ExpectedError::MissingTeam {
        let finops = fixture.finops.as_ref().expect("the seed carries finops");
        let error = finops.validate(&fixture).expect_err("the seed is rejected");
        assert!(
            error_matches(&error, expected),
            "unexpected finops error {error:?}"
        );
        return;
    }
    if expected == ExpectedError::Accepted {
        fixture.validate().expect("the finops seed validates");
        return;
    }
    let error = fixture.validate().expect_err("the finops seed is rejected");
    let FixtureError::FinOps(source) = error else {
        panic!("expected a finops fixture error, got {error:?}");
    };
    assert!(
        error_matches(&source, expected),
        "unexpected finops error {source:?}"
    );
}
