#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::dashboard::payload::JsonPayload;
use betterlite_postgres_fixture::dashboard::{DashboardFixtureError, SpendLogSeed};
use betterlite_postgres_fixture::error::FixtureError;
use betterlite_postgres_fixture::seed::{DatabaseFixture, FixtureSeed};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::NoTls;

use crate::OracleError;
use crate::case::{Case, decode_case};
use crate::runner::DatabaseEnvironment;

const CASE_PREFIX: &str = "name: dashboard-http\nconfig: config\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ndatabase:\n  seed:\n";

const CASE_SUFFIX: &str =
    "\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n";

const VALID_SEED: &str = "    users: [{user_id: alice}, {user_id: bob}]\n    keys: [{token: sk-alice-token, user_id: alice}, {token: sk-bob-token, user_id: bob}]\n    dashboard:\n      logs:\n        - {request_id: req-alice, call_type: acompletion, start_time: '2026-01-01T10:00:00.123', end_time: '2026-01-01T10:00:02.456', key_index: 0, completion_start_time: '2026-01-01T10:00:01.000', request_duration_ms: 2333, model: openai/gpt-5, model_id: model-id, model_group: default, custom_llm_provider: openai, user: alice, team_id: team-red, end_user: customer, session_id: session-shared, status: success, cache_hit: 'True', litellm_call_id: call-alice, spend: '1.25', total_tokens: 100, prompt_tokens: 60, completion_tokens: 40, metadata: null, request_tags: [tag-a, tag-b], messages: [{role: user, content: hello}], response: {id: chatcmpl-1}, proxy_server_request: {url: 'http://localhost/v1/chat/completions'}}\n        - {request_id: req-bob, call_type: acompletion, start_time: '2026-01-01T11:00:00.000', end_time: '2026-01-01T11:00:01.000', key_index: 1, session_id: session-shared, spend: '-0.5', metadata: {project_id: project-x}}\n";

const ABSENT_SEED: &str = "    users: [{user_id: alice}]\n";

const EMPTY_SEED: &str = "    users: [{user_id: alice}]\n    dashboard: {logs: []}\n";

fn case_with(seed: &str) -> String {
    format!("{CASE_PREFIX}{seed}{CASE_SUFFIX}")
}

fn expected_log(request_id: &str, key_index: usize) -> SpendLogSeed {
    SpendLogSeed {
        request_id: request_id.to_owned(),
        call_type: "acompletion".to_owned(),
        start_time: "2026-01-01T11:00:00.000".to_owned(),
        end_time: "2026-01-01T11:00:01.000".to_owned(),
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
        session_id: Some("session-shared".to_owned()),
        status: None,
        cache_hit: None,
        litellm_call_id: None,
        spend: Decimal::new(-5, 1),
        total_tokens: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        metadata: Some(JsonPayload::Object(BTreeMap::from([(
            "project_id".to_owned(),
            JsonPayload::Text("project-x".to_owned()),
        )]))),
        request_tags: None,
        messages: None,
        response: None,
        proxy_server_request: None,
    }
}

fn expected_messages() -> JsonPayload {
    JsonPayload::Array(vec![JsonPayload::Object(BTreeMap::from([
        ("role".to_owned(), JsonPayload::Text("user".to_owned())),
        ("content".to_owned(), JsonPayload::Text("hello".to_owned())),
    ]))])
}

fn expected_alice() -> SpendLogSeed {
    SpendLogSeed {
        request_id: "req-alice".to_owned(),
        call_type: "acompletion".to_owned(),
        start_time: "2026-01-01T10:00:00.123".to_owned(),
        end_time: "2026-01-01T10:00:02.456".to_owned(),
        key_index: 0,
        completion_start_time: Some("2026-01-01T10:00:01.000".to_owned()),
        request_duration_ms: Some(2333_i32),
        model: Some("openai/gpt-5".to_owned()),
        model_id: Some("model-id".to_owned()),
        model_group: Some("default".to_owned()),
        custom_llm_provider: Some("openai".to_owned()),
        user: Some("alice".to_owned()),
        team_id: Some("team-red".to_owned()),
        end_user: Some("customer".to_owned()),
        session_id: Some("session-shared".to_owned()),
        status: Some("success".to_owned()),
        cache_hit: Some("True".to_owned()),
        litellm_call_id: Some("call-alice".to_owned()),
        spend: Decimal::new(125, 2),
        total_tokens: 100_i32,
        prompt_tokens: 60_i32,
        completion_tokens: 40_i32,
        metadata: None,
        request_tags: Some(JsonPayload::Array(vec![
            JsonPayload::Text("tag-a".to_owned()),
            JsonPayload::Text("tag-b".to_owned()),
        ])),
        messages: Some(expected_messages()),
        response: Some(JsonPayload::Object(BTreeMap::from([(
            "id".to_owned(),
            JsonPayload::Text("chatcmpl-1".to_owned()),
        )]))),
        proxy_server_request: Some(JsonPayload::Object(BTreeMap::from([(
            "url".to_owned(),
            JsonPayload::Text("http://localhost/v1/chat/completions".to_owned()),
        )]))),
    }
}

fn expected_logs() -> Vec<SpendLogSeed> {
    vec![expected_alice(), expected_log("req-bob", 1)]
}

fn decoded_dashboard(case: &Case) -> Option<&[SpendLogSeed]> {
    case.database
        .as_ref()?
        .seed
        .dashboard
        .as_ref()
        .map(|dashboard| dashboard.logs.as_slice())
}

fn configured_url() -> Option<SecretString> {
    let environment =
        <DatabaseEnvironment as clap::Parser>::try_parse_from([env!("CARGO_PKG_NAME")])
            .expect("the database environment parses");
    environment.postgres_test_url
}

async fn provision_one(admin_url: &SecretString, database: &DatabaseFixture) -> PostgresFixture {
    PostgresFixture::provision_with_name_generator(
        Some(admin_url),
        &database.seed,
        &mut RandomDatabaseName,
    )
    .resolve()
    .await
    .expect("an independent dashboard database provisions")
}

async fn fixture_state(fixture: &PostgresFixture) -> Vec<String> {
    let (client, connection) = tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
        .await
        .expect("the provisioned database accepts connections");
    let mut drivers = JoinSet::new();
    drivers.spawn(async move { connection.await.expect("the fixture driver stays up") });
    let statement = "SELECT concat_ws('|', \"request_id\", \"api_key\", \
        coalesce(\"session_id\", '<null>'), \"spend\"::text, \
        coalesce(\"metadata\"::text, '<null>')) \
        FROM \"LiteLLM_SpendLogs\" ORDER BY \"request_id\"";
    let state = client
        .query(statement, &[])
        .await
        .expect("the dashboard rows are seeded")
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect();
    drop(client);
    drivers
        .join_next()
        .await
        .expect("the fixture driver joined")
        .expect("the fixture driver finished");
    state
}

fn expected_state() -> Vec<String> {
    vec![
        format!(
            "req-alice|{}|session-shared|1.25|{{}}",
            FixtureSeed::key_hash("sk-alice-token")
        ),
        format!(
            "req-bob|{}|session-shared|-0.5|{{\"project_id\": \"project-x\"}}",
            FixtureSeed::key_hash("sk-bob-token")
        ),
    ]
}

async fn provision_both(case: &Case) {
    let database = case
        .database
        .as_ref()
        .expect("the case carries a database fixture");
    let admin_url = configured_url().expect("POSTGRES_TEST_URL is set for the fixture test");
    let first = provision_one(&admin_url, database).await;
    let second = provision_one(&admin_url, database).await;
    assert_eq!(fixture_state(&first).await, expected_state());
    assert_eq!(fixture_state(&second).await, expected_state());
    first.close_async().await.expect("the first fixture closes");
    second
        .close_async()
        .await
        .expect("the second fixture closes");
}

#[rstest]
#[case::absent(case_with(ABSENT_SEED), Expected::Absent)]
#[case::empty(case_with(EMPTY_SEED), Expected::Empty)]
#[case::populated(case_with(VALID_SEED), Expected::Populated)]
#[case::provisions_both(case_with(VALID_SEED), Expected::ProvisionsBoth)]
#[tokio::test]
async fn decodes_dashboard_fixture(#[case] yaml: String, #[case] expected: Expected) {
    let outcome = decode_case(Path::new("<bytes>"), yaml.as_bytes());
    match expected {
        Expected::Absent => {
            let case = outcome.expect("the absent case decodes");
            assert!(decoded_dashboard(&case).is_none());
        }
        Expected::Empty => {
            let case = outcome.expect("the empty case decodes");
            let empty: &[SpendLogSeed] = &[];
            assert_eq!(decoded_dashboard(&case), Some(empty));
        }
        Expected::Populated => {
            let case = outcome.expect("the populated case decodes");
            assert_eq!(decoded_dashboard(&case), Some(expected_logs().as_slice()));
        }
        Expected::ProvisionsBoth => {
            let case = outcome.expect("the populated case decodes");
            provision_both(&case).await;
        }
    }
}

enum Expected {
    Absent,
    Empty,
    Populated,
    ProvisionsBoth,
}

fn assert_corpus_outcome(name: &str, result: &Result<Case, OracleError>) {
    match name {
        "dashboard-fixture-unknown-field.yaml" => assert!(
            matches!(result, Err(OracleError::CaseParse { .. })),
            "{name} should fail parsing"
        ),
        "dashboard-fixture-bad-index.yaml" => assert!(matches!(
            result,
            Err(OracleError::HttpCaseFixture {
                source: FixtureError::Dashboard(DashboardFixtureError::UnknownKey { .. })
            })
        )),
        "dashboard-fixture-duplicate-id.yaml" => assert!(matches!(
            result,
            Err(OracleError::HttpCaseFixture {
                source: FixtureError::Dashboard(DashboardFixtureError::DuplicateRequestId { .. })
            })
        )),
        "dashboard-fixture-oversized-json.yaml" => assert!(matches!(
            result,
            Err(OracleError::HttpCaseFixture {
                source: FixtureError::Dashboard(DashboardFixtureError::JsonTooLong { .. })
            })
        )),
        "dashboard-fixture-excess-rows.yaml" => assert!(matches!(
            result,
            Err(OracleError::HttpCaseFixture {
                source: FixtureError::Dashboard(DashboardFixtureError::TooManyLogs { .. })
            })
        )),
        other => panic!("unexpected dashboard corpus seed {other}"),
    }
}

#[test]
fn decodes_dashboard_fixture_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/http_case");
    let mut paths = fs::read_dir(root)
        .expect("the http case corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("dashboard-fixture-"))
        })
        .collect::<Vec<PathBuf>>();
    paths.sort();
    assert_eq!(
        paths.len(),
        5,
        "the dashboard fixture corpus carries its five seeds"
    );
    paths
        .into_iter()
        .try_for_each(|path| {
            let bytes = fs::read(&path).expect("the corpus seed reads");
            let result = decode_case(Path::new("<bytes>"), &bytes);
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .expect("the corpus seed is named");
            assert_corpus_outcome(name, &result);
            Ok::<(), ()>(())
        })
        .expect("every dashboard corpus seed is rejected");
}
