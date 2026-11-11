#![cfg(test)]

use std::path::Path;

use betterlite_postgres_fixture::daily_usage::{DailyUsageError, DailyUsageRow, DailyUsageTable};
use betterlite_postgres_fixture::error::FixtureError;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::OracleError;
use crate::case::{Case, decode_case};

const CASE_PREFIX: &str = "name: usage-activity\nconfig: config\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ndatabase:\n  seed:\n";

const CASE_SUFFIX: &str =
    "\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n";

const VALID_SEED: &str = "    users: [{user_id: alice}, {user_id: bob}]\n    keys: [{token: sk-alice, user_id: alice}, {token: sk-bob, user_id: bob}]\n    daily_usage:\n      - {table: user, id: shared-row, entity_id: alice, date: '2025-01-01', key_index: 0, model: openai/gpt-5, custom_llm_provider: openai, model_group: default, spend: 1.25, prompt_tokens: 100, completion_tokens: 50, api_requests: 2, successful_requests: 1, failed_requests: 1}\n      - {table: team, id: shared-row, entity_id: team-red, date: '2025-02-02', key_index: 1, model: null, custom_llm_provider: null, spend: -0.5}\n      - {table: tag, id: shared-row, entity_id: tag-blue, date: '2025-03-03', key_index: 0, model: openai/gpt-5, custom_llm_provider: openai, prompt_tokens: 7, completion_tokens: 3}\n";

const ABSENT_SEED: &str = "    users: [{user_id: alice}]\n";

const EMPTY_SEED: &str = "    users: [{user_id: alice}]\n    daily_usage: []\n";

const MISSING_KEY_INDEX_SEED: &str = "    daily_usage:\n      - {table: user, id: row, entity_id: alice, date: '2025-01-01', model: null, custom_llm_provider: null}\n";

const INVALID_TABLE_SEED: &str = "    daily_usage:\n      - {table: project, id: row, entity_id: alice, date: '2025-01-01', key_index: 0, model: null, custom_llm_provider: null}\n";

const UNKNOWN_FIELD_SEED: &str = "    daily_usage:\n      - {table: user, id: row, entity_id: alice, date: '2025-01-01', key_index: 0, model: null, custom_llm_provider: null, region: eu}\n";

const DUPLICATE_SEED: &str = "    users: [{user_id: alice}]\n    keys: [{token: sk-alice, user_id: alice}]\n    daily_usage:\n      - {table: user, id: row, entity_id: alice, date: '2025-01-01', key_index: 0, model: null, custom_llm_provider: null}\n      - {table: user, id: row, entity_id: bob, date: '2025-01-02', key_index: 0, model: null, custom_llm_provider: null}\n";

const UNKNOWN_KEY_SEED: &str = "    users: [{user_id: alice}]\n    keys: [{token: sk-alice, user_id: alice}]\n    daily_usage:\n      - {table: user, id: row, entity_id: alice, date: '2025-01-01', key_index: 1, model: null, custom_llm_provider: null}\n";

const NEGATIVE_COUNTER_SEED: &str = "    users: [{user_id: alice}]\n    keys: [{token: sk-alice, user_id: alice}]\n    daily_usage:\n      - {table: user, id: row, entity_id: alice, date: '2025-01-01', key_index: 0, model: null, custom_llm_provider: null, failed_requests: -1}\n";

#[derive(Clone, Copy)]
enum DailyUsageKind {
    TooManyRows,
    DuplicateRow,
    UnknownKey,
    NegativeCounter,
}

enum Expected {
    Absent,
    Empty,
    Rows(Vec<DailyUsageRow>),
    ParseError,
    Fixture(DailyUsageKind),
}

fn case_with(seed: &str) -> String {
    format!("{CASE_PREFIX}{seed}{CASE_SUFFIX}")
}

fn oversized_case() -> String {
    let rows = (0_usize..=256)
        .map(|index| {
            format!(
                "      - {{table: user, id: row-{index}, entity_id: alice, date: '2025-01-01', key_index: 0, model: null, custom_llm_provider: null}}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    case_with(&format!("    daily_usage:\n{rows}\n"))
}

fn fixture_row(table: DailyUsageTable) -> DailyUsageRow {
    DailyUsageRow {
        table,
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

fn expected_rows() -> Vec<DailyUsageRow> {
    let mut user = fixture_row(DailyUsageTable::User);
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

    let mut team = fixture_row(DailyUsageTable::Team);
    team.id = "shared-row".to_owned();
    team.entity_id = Some("team-red".to_owned());
    team.date = "2025-02-02".to_owned();
    team.key_index = 1;
    team.spend = Decimal::new(-5, 1);

    let mut tag = fixture_row(DailyUsageTable::Tag);
    tag.id = "shared-row".to_owned();
    tag.entity_id = Some("tag-blue".to_owned());
    tag.date = "2025-03-03".to_owned();
    tag.model = Some("openai/gpt-5".to_owned());
    tag.custom_llm_provider = Some("openai".to_owned());
    tag.prompt_tokens = 7;
    tag.completion_tokens = 3;
    vec![user, team, tag]
}

fn matches_kind(error: &DailyUsageError, kind: DailyUsageKind) -> bool {
    matches!(
        (error, kind),
        (
            DailyUsageError::TooManyRows { .. },
            DailyUsageKind::TooManyRows
        ) | (
            DailyUsageError::DuplicateRow { .. },
            DailyUsageKind::DuplicateRow
        ) | (
            DailyUsageError::UnknownKey { .. },
            DailyUsageKind::UnknownKey
        ) | (
            DailyUsageError::NegativeCounter { .. },
            DailyUsageKind::NegativeCounter
        )
    )
}

fn assert_fixture_error(outcome: Result<Case, OracleError>, kind: DailyUsageKind) {
    let Err(OracleError::HttpCaseFixture {
        source: FixtureError::DailyUsage(source),
    }) = outcome
    else {
        panic!("expected a daily usage fixture error");
    };
    assert!(matches_kind(&source, kind));
}

fn decoded_seed(case: &Case) -> Option<&[DailyUsageRow]> {
    case.database.as_ref()?.seed.daily_usage.as_deref()
}

#[rstest]
#[case::valid(case_with(VALID_SEED), Expected::Rows(expected_rows()))]
#[case::absent(case_with(ABSENT_SEED), Expected::Absent)]
#[case::empty(case_with(EMPTY_SEED), Expected::Empty)]
#[case::missing_key_index(case_with(MISSING_KEY_INDEX_SEED), Expected::ParseError)]
#[case::invalid_table(case_with(INVALID_TABLE_SEED), Expected::ParseError)]
#[case::unknown_field(case_with(UNKNOWN_FIELD_SEED), Expected::ParseError)]
#[case::oversized(oversized_case(), Expected::Fixture(DailyUsageKind::TooManyRows))]
#[case::duplicate_rows(
    case_with(DUPLICATE_SEED),
    Expected::Fixture(DailyUsageKind::DuplicateRow)
)]
#[case::unknown_key(
    case_with(UNKNOWN_KEY_SEED),
    Expected::Fixture(DailyUsageKind::UnknownKey)
)]
#[case::negative_counter(
    case_with(NEGATIVE_COUNTER_SEED),
    Expected::Fixture(DailyUsageKind::NegativeCounter)
)]
fn decodes_daily_usage_seed(#[case] yaml: String, #[case] expected: Expected) {
    let outcome = decode_case(Path::new("<bytes>"), yaml.as_bytes());
    match expected {
        Expected::ParseError => assert!(matches!(outcome, Err(OracleError::CaseParse { .. }))),
        Expected::Fixture(kind) => assert_fixture_error(outcome, kind),
        Expected::Rows(rows) => {
            let case = outcome.expect("case decodes");
            assert_eq!(decoded_seed(&case), Some(rows.as_slice()));
        }
        Expected::Absent => {
            let case = outcome.expect("case decodes");
            assert!(decoded_seed(&case).is_none());
        }
        Expected::Empty => {
            let case = outcome.expect("case decodes");
            let empty: &[DailyUsageRow] = &[];
            assert_eq!(decoded_seed(&case), Some(empty));
        }
    }
}
