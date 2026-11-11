#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::error::FixtureError;
use betterlite_postgres_fixture::finops::FinOpsFixtureError;
use betterlite_postgres_fixture::finops::FinOpsSeed;
use betterlite_postgres_fixture::finops::model::{
    ConfigName, ConfigRow, FinOpsUsageRow, KeyMetadataRow, OrganizationRow, UsageApiKey,
};
use betterlite_postgres_fixture::seed::DatabaseFixture;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::NoTls;

use crate::OracleError;
use crate::case::{Case, decode_case};
use crate::runner::DatabaseEnvironment;

const CASE_PREFIX: &str = "name: finops-http\nconfig: config\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ndatabase:\n  seed:\n";

const CASE_SUFFIX: &str =
    "\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n";

const ABSENT_SEED: &str = "    users: [{user_id: alice}]\n";

const EMPTY_SEED: &str = "    users: [{user_id: alice}]\n    finops: {}\n";

const UNKNOWN_FIELD_SEED: &str =
    "    users: [{user_id: alice}]\n    finops: {unexpected_list: []}\n";

const DUPLICATE_SEED: &str = "    users: [{user_id: alice}]\n    budgets: [{budget_id: budget-1, user_id: alice}]\n    finops:\n      organizations:\n        - {organization_id: org-key, organization_alias: Key Org, budget_id: budget-1, created_by: alice, updated_by: alice}\n        - {organization_id: org-key, organization_alias: Key Org, budget_id: budget-1, created_by: alice, updated_by: alice}\n";

const MISSING_REFERENCE_SEED: &str = "    users: [{user_id: alice}]\n    finops:\n      organizations:\n        - {organization_id: org-key, organization_alias: Key Org, budget_id: budget-missing, created_by: alice, updated_by: alice}\n";

const TRUNCATED_SEED: &str = "    users: [{user_id: alice}]\n    finops:\n      organizations: [{organization_id: org-key, organization_alias: Key Org\n";

const POPULATED_SEED: &str = "    users: [{user_id: alice}, {user_id: bob}]\n    keys: [{token: sk-alice-token, user_id: alice, team_id: team-red}, {token: sk-bob-token, user_id: bob, team_id: team-blue}]\n    teams: [{team_id: team-red, team_alias: Red Team}, {team_id: team-blue, team_alias: Blue Team}]\n    budgets: [{budget_id: budget-1, user_id: alice}]\n    finops:\n      organizations:\n        - {organization_id: org-key, organization_alias: Key Org, budget_id: budget-1, created_by: alice, updated_by: alice}\n        - {organization_id: org-team, organization_alias: Team Org, budget_id: budget-1, created_by: bob, updated_by: bob, models: [gpt-5, claude]}\n      key_metadata:\n        - {token: sk-alice-token, key_alias: alice-key, organization_id: org-key, team_organization_id: org-team}\n      daily_usage:\n        - {id: row-key, user_id: alice, date: '2025-01-01', api_key: {key: sk-alice-token}, model: openai/gpt-5, model_group: default, custom_llm_provider: openai, mcp_namespaced_tool_name: mcp__tool, endpoint: /v1/chat/completions, prompt_tokens: 10, completion_tokens: 5, cache_creation_input_tokens: 4, cache_read_input_tokens: 3, compression_saved_tokens: 2, compression_savings_spend: '0.5', prompt_caching_savings_spend: '0.25', gateway_injected_caching_savings_spend: '0.125', autorouter_savings_spend: '0.0625', spend: '1.25', api_requests: 2, successful_requests: 1, failed_requests: 1, total_response_time_ms: 120, timed_requests: 2, created_at: '2025-01-01T00:00:00', updated_at: '2025-01-01T00:00:05'}\n        - {id: row-orphan, date: '2025-01-02', api_key: {orphan_hash: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}, created_at: '2025-01-02T00:00:00', updated_at: '2025-01-02T00:00:00'}\n      config_rows:\n        - {name: vantage_settings, serialized_value: cipher-value}\n        - {name: cloudzero_settings}\n";

const ORPHAN_HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const VANTAGE_VALUE: &str = "cipher-value";

fn case_with(seed: &str) -> String {
    format!("{CASE_PREFIX}{seed}{CASE_SUFFIX}")
}

fn text(value: &str) -> String {
    value.to_owned()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn expected_organizations() -> Vec<OrganizationRow> {
    vec![
        OrganizationRow {
            organization_id: "org-key".to_owned(),
            organization_alias: "Key Org".to_owned(),
            budget_id: "budget-1".to_owned(),
            created_by: "alice".to_owned(),
            updated_by: "alice".to_owned(),
            models: Vec::new(),
        },
        OrganizationRow {
            organization_id: "org-team".to_owned(),
            organization_alias: "Team Org".to_owned(),
            budget_id: "budget-1".to_owned(),
            created_by: "bob".to_owned(),
            updated_by: "bob".to_owned(),
            models: vec!["gpt-5".to_owned(), "claude".to_owned()],
        },
    ]
}

fn expected_key_usage() -> FinOpsUsageRow {
    FinOpsUsageRow {
        id: "row-key".to_owned(),
        user_id: Some("alice".to_owned()),
        date: "2025-01-01".to_owned(),
        api_key: UsageApiKey::Key(secret("sk-alice-token")),
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

fn expected_orphan_usage() -> FinOpsUsageRow {
    let mut row = expected_key_usage();
    row.id = "row-orphan".to_owned();
    row.user_id = None;
    row.date = "2025-01-02".to_owned();
    row.api_key = UsageApiKey::OrphanHash(ORPHAN_HASH.to_owned());
    row.model = None;
    row.model_group = None;
    row.custom_llm_provider = None;
    row.mcp_namespaced_tool_name = None;
    row.endpoint = None;
    row.prompt_tokens = 0;
    row.completion_tokens = 0;
    row.cache_read_input_tokens = 0;
    row.cache_creation_input_tokens = 0;
    row.compression_saved_tokens = 0;
    row.compression_savings_spend = Decimal::ZERO;
    row.prompt_caching_savings_spend = Decimal::ZERO;
    row.gateway_injected_savings = Decimal::ZERO;
    row.autorouter_savings_spend = Decimal::ZERO;
    row.spend = Decimal::ZERO;
    row.api_requests = 0;
    row.successful_requests = 0;
    row.failed_requests = 0;
    row.total_response_time_ms = 0;
    row.timed_requests = 0;
    row.created_at = "2025-01-02T00:00:00".to_owned();
    row.updated_at = "2025-01-02T00:00:00".to_owned();
    row
}

fn expected_finops() -> FinOpsSeed {
    FinOpsSeed {
        organizations: expected_organizations(),
        key_metadata: vec![KeyMetadataRow {
            token: secret("sk-alice-token"),
            key_alias: Some("alice-key".to_owned()),
            organization_id: Some("org-key".to_owned()),
            team_organization_id: Some("org-team".to_owned()),
        }],
        daily_usage: vec![expected_key_usage(), expected_orphan_usage()],
        config_rows: vec![
            ConfigRow {
                name: ConfigName::VantageSettings,
                serialized_value: Some(secret(VANTAGE_VALUE)),
            },
            ConfigRow {
                name: ConfigName::CloudzeroSettings,
                serialized_value: None,
            },
        ],
    }
}

fn decoded_finops(case: &Case) -> Option<&FinOpsSeed> {
    case.database.as_ref()?.seed.finops.as_ref()
}

#[derive(Clone, Copy)]
enum FixtureKind {
    DuplicateOrganization,
    TooManyRows,
    MissingBudget,
}

enum Expected {
    Populated,
    Empty,
    Absent,
    ParseError,
    Fixture(FixtureKind),
    ProvisionsBoth,
}

fn assert_fixture_error(outcome: Result<Case, OracleError>, kind: FixtureKind) {
    let Err(OracleError::HttpCaseFixture {
        source: FixtureError::FinOps(source),
    }) = outcome
    else {
        panic!("expected a finops fixture error");
    };
    let matched = matches!(
        (&source, kind),
        (
            FinOpsFixtureError::DuplicateOrganization { .. },
            FixtureKind::DuplicateOrganization
        ) | (
            FinOpsFixtureError::TooManyRows { .. },
            FixtureKind::TooManyRows
        ) | (
            FinOpsFixtureError::MissingBudget { .. },
            FixtureKind::MissingBudget
        )
    );
    assert!(matched, "unexpected finops error {source:?}");
}

async fn fixture_state(fixture: &PostgresFixture) -> (i64, i64, i64) {
    let (client, connection) = tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
        .await
        .expect("the provisioned database accepts connections");
    let mut drivers = JoinSet::new();
    drivers.spawn(async move { connection.await.expect("the fixture driver stays up") });
    let daily: i64 = client
        .query_one("SELECT count(*) FROM \"LiteLLM_DailyUserSpend\"", &[])
        .await
        .expect("the daily usage rows are seeded")
        .get(0);
    let config: i64 = client
        .query_one("SELECT count(*) FROM \"LiteLLM_Config\"", &[])
        .await
        .expect("the config rows are seeded")
        .get(0);
    let organizations: i64 = client
        .query_one("SELECT count(*) FROM \"LiteLLM_OrganizationTable\"", &[])
        .await
        .expect("the organization rows are seeded")
        .get(0);
    drop(client);
    drivers
        .join_next()
        .await
        .expect("the fixture driver joined")
        .expect("the fixture driver finished");
    (daily, config, organizations)
}

fn configured_url() -> Option<SecretString> {
    let environment =
        <DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])
            .expect("the database environment parses");
    environment.postgres_test_url
}

async fn provision_one(
    admin_url: &SecretString,
    database: &DatabaseFixture,
    generator: &mut RandomDatabaseName,
) -> PostgresFixture {
    PostgresFixture::provision_with_name_generator(Some(admin_url), &database.seed, generator)
        .resolve()
        .await
        .expect("an independent fixture database provisions")
}

async fn provision_both(case: &Case) {
    let database = case
        .database
        .as_ref()
        .expect("the case carries a database fixture");
    let Some(admin_url) = configured_url() else {
        return;
    };
    let mut generator = RandomDatabaseName;
    let first = provision_one(&admin_url, database, &mut generator).await;
    let second = provision_one(&admin_url, database, &mut generator).await;
    let first_state = fixture_state(&first).await;
    let second_state = fixture_state(&second).await;
    assert_eq!(first_state, (2, 2, 2));
    assert_eq!(second_state, first_state);
    first.close_async().await.expect("the first fixture closes");
    second
        .close_async()
        .await
        .expect("the second fixture closes");
}

#[rstest]
#[case::valid_populated(case_with(POPULATED_SEED), Expected::Populated)]
#[case::valid_empty(case_with(EMPTY_SEED), Expected::Empty)]
#[case::absent(case_with(ABSENT_SEED), Expected::Absent)]
#[case::unknown_field(case_with(UNKNOWN_FIELD_SEED), Expected::ParseError)]
#[case::duplicate(
    case_with(DUPLICATE_SEED),
    Expected::Fixture(FixtureKind::DuplicateOrganization)
)]
#[case::excess_rows(
    text(include_str!("../../fuzz/corpus/http_case/finops/finops-excess-row.yaml")),
    Expected::Fixture(FixtureKind::TooManyRows)
)]
#[case::missing_reference(
    case_with(MISSING_REFERENCE_SEED),
    Expected::Fixture(FixtureKind::MissingBudget)
)]
#[case::truncated(case_with(TRUNCATED_SEED), Expected::ParseError)]
#[case::provisions_both(case_with(POPULATED_SEED), Expected::ProvisionsBoth)]
#[tokio::test]
async fn finops_http_fixture_decode_contract(#[case] yaml: String, #[case] expected: Expected) {
    let outcome = decode_case(Path::new("<bytes>"), yaml.as_bytes());
    match expected {
        Expected::ParseError => assert!(matches!(outcome, Err(OracleError::CaseParse { .. }))),
        Expected::Fixture(kind) => assert_fixture_error(outcome, kind),
        Expected::Populated => {
            let case = outcome.expect("the populated case decodes");
            assert_eq!(decoded_finops(&case), Some(&expected_finops()));
        }
        Expected::Empty => {
            let case = outcome.expect("the empty case decodes");
            assert_eq!(decoded_finops(&case), Some(&FinOpsSeed::default()));
        }
        Expected::Absent => {
            let case = outcome.expect("the absent case decodes");
            assert!(decoded_finops(&case).is_none());
        }
        Expected::ProvisionsBoth => {
            let case = outcome.expect("the populated case decodes");
            provision_both(&case).await;
        }
    }
}

fn assert_corpus_outcome(name: &str, path: &Path, result: &Result<Case, OracleError>) {
    match name {
        "finops-valid-empty.yaml" | "finops-valid-populated.yaml" => {
            assert!(result.is_ok(), "{} should be accepted", path.display());
        }
        "finops-unknown-field.yaml" | "finops-truncated-input.yaml" => {
            assert!(
                matches!(result, Err(OracleError::CaseParse { .. })),
                "{} should fail parsing",
                path.display()
            );
        }
        "finops-duplicate.yaml" => assert!(matches!(
            result,
            Err(OracleError::HttpCaseFixture {
                source: FixtureError::FinOps(FinOpsFixtureError::DuplicateOrganization { .. })
            })
        )),
        "finops-excess-row.yaml" => assert!(matches!(
            result,
            Err(OracleError::HttpCaseFixture {
                source: FixtureError::FinOps(FinOpsFixtureError::TooManyRows { .. })
            })
        )),
        "finops-missing-reference.yaml" => assert!(matches!(
            result,
            Err(OracleError::HttpCaseFixture {
                source: FixtureError::FinOps(FinOpsFixtureError::MissingBudget { .. })
            })
        )),
        _ => panic!("unexpected corpus seed {}", path.display()),
    }
}

#[test]
fn replays_the_finops_http_fixture_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/http_case/finops");
    let mut paths = fs::read_dir(root)
        .expect("the finops corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "the finops corpus has seeds");
    paths
        .into_iter()
        .try_for_each(|path| {
            let bytes = fs::read(&path).expect("the corpus seed reads");
            let result = decode_case(Path::new("<bytes>"), &bytes);
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .expect("the corpus seed is named");
            assert_corpus_outcome(name, &path, &result);
            Ok::<(), ()>(())
        })
        .expect("every corpus seed replays");
}
