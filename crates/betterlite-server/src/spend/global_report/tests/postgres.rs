use std::collections::BTreeMap;

use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, TimeZone as _, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::spend::global_report::rows::GlobalReport;
use crate::spend::global_report::store::{
    GlobalReportStore as _, PostgresGlobalReportStore, ReportBranch, ReportWindow,
};
use crate::spend::reports::scope::ScopeColumn;
use crate::spend::reports::store::PostgresScopedReportStore;

const SCHEMA_SQL: &str = r#"
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id TEXT PRIMARY KEY,
    api_key TEXT NOT NULL DEFAULT '',
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    "startTime" TIMESTAMP(3) NOT NULL,
    model TEXT NOT NULL DEFAULT '',
    "user" TEXT,
    team_id TEXT,
    end_user TEXT
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    team_alias TEXT
);
INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias) VALUES
    ('t1', 'Alpha'),
    ('t2', 'Alpha'),
    ('t3', NULL);
INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, total_tokens, prompt_tokens, completion_tokens, "startTime", model, "user", team_id, end_user) VALUES
    ('before', 'k1', 1000.0, 10, 6, 4, '2023-12-31 23:59:59', 'gpt', 'u1', 't1', 'c1'),
    ('start', 'k1', 1.0, 3, 1, 2, '2024-01-01 00:00:00', 'gpt', 'u1', 't1', 'c1'),
    ('start-claude', 'k1', 2.0, 5, 2, 3, '2024-01-01 08:00:00', 'claude', 'u1', 't1', 'c1'),
    ('other-customer', 'k2', 4.0, 7, 3, 4, '2024-01-01 09:00:00', 'gpt', 'u2', 't1', 'c2'),
    ('other-team', 'k3', 8.0, 9, 4, 5, '2024-01-01 10:00:00', 'gpt', 'u2', 't2', 'c1'),
    ('alias-less', 'k6', 128.0, 17, 8, 9, '2024-01-01 11:00:00', 'gpt', 'u3', 't3', 'c1'),
    ('end', 'k1', 16.0, 11, 5, 6, '2024-01-02 23:59:59', 'gpt', 'u1', 't1', 'c1'),
    ('no-customer', 'k4', 32.0, 13, 6, 7, '2024-01-02 12:00:00', 'gpt', NULL, 't3', NULL),
    ('no-team', 'k5', 64.0, 15, 7, 8, '2024-01-02 13:00:00', 'gpt', NULL, NULL, 'c3'),
    ('ghost-team', 'k7', 256.0, 19, 9, 10, '2024-01-02 14:00:00', 'gpt', 'u3', 't9', 'c1'),
    ('after', 'k1', 2000.0, 10, 6, 4, '2024-01-03 00:00:00', 'gpt', 'u1', 't1', 'c1');
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, PostgresGlobalReportStore) {
    let url = Environment::parse_from(["global-report-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![String::from(SCHEMA_SQL)])
            .resolve()
            .await
            .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let store = PostgresGlobalReportStore {
        reports: PostgresScopedReportStore {
            store: PostgresStore { pool: Some(pool) },
        },
    };
    (fixture, store)
}

fn midnight(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, day, 0, 0, 0).unwrap()
}

async fn report(
    store: &PostgresGlobalReportStore,
    branch: ReportBranch<'_>,
    last_day: u32,
) -> GlobalReport {
    let start = midnight(1);
    let end = midnight(last_day);
    store
        .report(
            branch,
            ReportWindow {
                start: &start,
                end: &end,
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn team_customer_rows_need_both_filters_and_keep_the_window() {
    let (_fixture, store) = database().await;
    let branch = ReportBranch::TeamCustomer {
        team_id: "t1",
        customer_id: "c1",
    };
    let GlobalReport::TeamsCustomers(days) = report(&store, branch, 2).await else {
        panic!("the team and customer branch answers teams_customers rows");
    };
    let shape: Vec<(&str, &str, &str, String)> = days
        .iter()
        .flat_map(|day| {
            day.teams_customers.iter().map(move |group| {
                let spend = group.total_spend.to_string();
                (
                    day.group_by_day.as_str(),
                    group.team_name.as_str(),
                    group.customer.as_str(),
                    spend,
                )
            })
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            ("2024-01-01T00:00:00.000Z", "Alpha", "c1", String::from("3")),
            (
                "2024-01-02T00:00:00.000Z",
                "Alpha",
                "c1",
                String::from("16")
            ),
        ]
    );
}

#[tokio::test]
async fn team_customer_metadata_lists_each_model_and_key() {
    let (_fixture, store) = database().await;
    let branch = ReportBranch::TeamCustomer {
        team_id: "t1",
        customer_id: "c1",
    };
    let GlobalReport::TeamsCustomers(days) = report(&store, branch, 1).await else {
        panic!("the team and customer branch answers teams_customers rows");
    };
    // SQL aggregates the nested metadata in no promised order, so it is compared as a map
    let first_day: BTreeMap<String, String> = days[0].teams_customers[0]
        .metadata
        .iter()
        .map(|entry| {
            let spent = format!("{}:{}", entry.spend, entry.total_tokens);
            (format!("{}/{}", entry.model, entry.api_key), spent)
        })
        .collect();
    assert_eq!(
        first_day,
        BTreeMap::from([
            (String::from("claude/k1"), String::from("2:5")),
            (String::from("gpt/k1"), String::from("1:3")),
        ])
    );
}

#[rstest]
#[case::null_alias("t3", "2024-01-01T00:00:00.000Z", "128")]
#[case::no_team_row("t9", "2024-01-02T00:00:00.000Z", "256")]
#[tokio::test]
async fn team_customer_rows_without_an_alias_are_unassigned(
    #[case] team_id: &str,
    #[case] expected_day: &str,
    #[case] expected_spend: &str,
) {
    let (_fixture, store) = database().await;
    let branch = ReportBranch::TeamCustomer {
        team_id,
        customer_id: "c1",
    };
    let GlobalReport::TeamsCustomers(days) = report(&store, branch, 2).await else {
        panic!("the team and customer branch answers teams_customers rows");
    };
    assert_eq!(days.len(), 1);
    assert_eq!(days[0].group_by_day, expected_day);
    assert_eq!(days[0].teams_customers.len(), 1);
    assert_eq!(days[0].teams_customers[0].team_name, "Unassigned Team");
    assert_eq!(
        days[0].teams_customers[0].total_spend.to_string(),
        expected_spend
    );
}

#[tokio::test]
async fn team_rows_merge_teams_that_share_an_alias() {
    let (_fixture, store) = database().await;
    let GlobalReport::Teams(days) = report(&store, ReportBranch::Teams(None), 2).await else {
        panic!("the team branch answers team rows");
    };
    let totals: Vec<(String, BTreeMap<String, String>)> = days
        .iter()
        .map(|day| {
            let teams = day
                .teams
                .iter()
                .map(|team| (team.team_name.clone(), team.total_spend.to_string()))
                .collect();
            (day.group_by_day.clone(), teams)
        })
        .collect();
    assert_eq!(
        totals,
        vec![
            (
                String::from("2024-01-01T00:00:00.000Z"),
                BTreeMap::from([
                    (String::from("Alpha"), String::from("15")),
                    (String::from("Unassigned Team"), String::from("128")),
                ])
            ),
            (
                String::from("2024-01-02T00:00:00.000Z"),
                BTreeMap::from([
                    (String::from("Alpha"), String::from("16")),
                    (String::from("Unassigned Team"), String::from("352")),
                ])
            ),
        ]
    );
}

#[tokio::test]
async fn customer_rows_group_every_customer_by_day() {
    let (_fixture, store) = database().await;
    let GlobalReport::Customers(days) = report(&store, ReportBranch::Customers, 2).await else {
        panic!("the customer branch answers customer rows");
    };
    let totals: Vec<(String, BTreeMap<Option<String>, String>)> = days
        .iter()
        .map(|day| {
            let customers = day
                .customers
                .iter()
                .map(|group| (group.customer.clone(), group.total_spend.to_string()))
                .collect();
            (day.group_by_day.clone(), customers)
        })
        .collect();
    assert_eq!(
        totals,
        vec![
            (
                String::from("2024-01-01T00:00:00.000Z"),
                BTreeMap::from([
                    (Some(String::from("c1")), String::from("139")),
                    (Some(String::from("c2")), String::from("4")),
                ])
            ),
            (
                String::from("2024-01-02T00:00:00.000Z"),
                BTreeMap::from([
                    (None, String::from("32")),
                    (Some(String::from("c1")), String::from("272")),
                    (Some(String::from("c3")), String::from("64")),
                ])
            ),
        ]
    );
}

#[tokio::test]
async fn key_rows_rank_every_key_by_total_cost() {
    let (_fixture, store) = database().await;
    let GlobalReport::Keys(rows) = report(&store, ReportBranch::Keys, 2).await else {
        panic!("the api_key group answers key rows");
    };
    let ranked: Vec<(&str, String)> = rows
        .iter()
        .map(|row| (row.api_key.as_str(), row.total_cost.to_string()))
        .collect();
    assert_eq!(
        ranked,
        vec![
            ("k7", String::from("256")),
            ("k6", String::from("128")),
            ("k5", String::from("64")),
            ("k4", String::from("32")),
            ("k1", String::from("19")),
            ("k3", String::from("8")),
            ("k2", String::from("4")),
        ]
    );
    let k1 = &rows[4];
    assert_eq!((k1.total_input_tokens, k1.total_output_tokens), (8, 11));
    let mut models: Vec<(&str, String)> = k1
        .model_details
        .iter()
        .map(|detail| (detail.model.as_str(), detail.total_cost.to_string()))
        .collect();
    models.sort_unstable();
    assert_eq!(
        models,
        vec![("claude", String::from("2")), ("gpt", String::from("17"))]
    );
}

#[tokio::test]
async fn scoped_rows_filter_by_the_key_or_the_user() {
    let (_fixture, store) = database().await;
    let by_user = ReportBranch::Scoped {
        column: ScopeColumn::User,
        value: "u2",
    };
    let GlobalReport::Keys(user_rows) = report(&store, by_user, 2).await else {
        panic!("the user branch answers key rows");
    };
    let user_keys: Vec<(&str, String)> = user_rows
        .iter()
        .map(|row| (row.api_key.as_str(), row.total_cost.to_string()))
        .collect();
    assert_eq!(
        user_keys,
        vec![("k3", String::from("8")), ("k2", String::from("4"))]
    );
    let by_key = ReportBranch::Scoped {
        column: ScopeColumn::ApiKey,
        value: "k1",
    };
    let GlobalReport::Keys(key_rows) = report(&store, by_key, 1).await else {
        panic!("the key branch answers key rows");
    };
    assert_eq!(key_rows.len(), 1);
    assert_eq!(key_rows[0].total_cost.to_string(), "3");
}
