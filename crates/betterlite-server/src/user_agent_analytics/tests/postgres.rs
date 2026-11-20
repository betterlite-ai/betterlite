use axum::http::StatusCode;
use rstest::rstest;

use crate::user_agent_analytics::tests::support::ActiveUserBody;
use crate::user_agent_analytics::tests::support::ActiveUsersBody;
use crate::user_agent_analytics::tests::support::DetailBody;
use crate::user_agent_analytics::tests::support::DistinctBody;
use crate::user_agent_analytics::tests::support::caller;
use crate::user_agent_analytics::tests::support::close;
use crate::user_agent_analytics::tests::support::database;
use crate::user_agent_analytics::tests::support::get;
use crate::user_agent_analytics::tests::support::postgres_app;
use crate::user_agent_analytics::tests::support::seed;

const INSERT_DISTINCT_ROWS: &str = r#"INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-01', 'sk-a', CURRENT_TIMESTAMP),
 ('2', 'User-Agent:alpha', '2026-01-02', 'sk-b', CURRENT_TIMESTAMP),
 ('3', 'User-Agent:alpha', '2026-01-03', 'sk-c', CURRENT_TIMESTAMP),
 ('4', 'User-Agent:beta', '2020-01-01', 'sk-old-one', CURRENT_TIMESTAMP),
 ('5', 'User-Agent:beta', '2020-01-02', 'sk-old-two', CURRENT_TIMESTAMP),
 ('6', 'plain-tag', '2026-01-03', 'sk-d', CURRENT_TIMESTAMP),
 ('7', 'team:alpha', '2026-01-03', 'sk-d', CURRENT_TIMESTAMP),
 ('8', NULL, '2026-01-03', 'sk-d', CURRENT_TIMESTAMP);"#;

const INSERT_MANY_TAGS: &str = r#"INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at)
 SELECT 'bulk-' || g, 'User-Agent:agent-' || g, '2026-01-05', 'sk-bulk', CURRENT_TIMESTAMP FROM generate_series(1, 252) AS g;"#;

const INSERT_DAILY_WINDOW: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-2'), ('sk-c', 'user-3'), ('sk-d', 'user-4');
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-03', 'sk-a', CURRENT_TIMESTAMP),
 ('2', 'User-Agent:alpha', '2026-01-04', 'sk-b', CURRENT_TIMESTAMP),
 ('3', 'User-Agent:alpha', '2026-01-11', 'sk-c', CURRENT_TIMESTAMP),
 ('4', 'User-Agent:alpha', '2026-01-12', 'sk-d', CURRENT_TIMESTAMP);"#;

const INSERT_DAILY_USERS: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-1'), ('sk-c', 'user-2'), ('sk-null', NULL);
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-05', 'sk-a', CURRENT_TIMESTAMP),
 ('2', 'User-Agent:alpha', '2026-01-05', 'sk-b', CURRENT_TIMESTAMP),
 ('3', 'User-Agent:alpha', '2026-01-05', 'sk-c', CURRENT_TIMESTAMP),
 ('4', 'User-Agent:alpha', '2026-01-05', 'sk-null', CURRENT_TIMESTAMP),
 ('5', 'User-Agent:alpha', '2026-01-05', 'sk-missing', CURRENT_TIMESTAMP);"#;

const INSERT_DAILY_EXACT: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-2'), ('sk-c', 'user-3'), ('sk-d', 'user-4');
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-05', 'sk-a', CURRENT_TIMESTAMP),
 ('2', 'User-Agent:alpha', '2026-01-05', 'sk-b', CURRENT_TIMESTAMP),
 ('3', 'User-Agent:beta', '2026-01-05', 'sk-c', CURRENT_TIMESTAMP),
 ('4', 'User-Agent:gamma', '2026-01-05', 'sk-d', CURRENT_TIMESTAMP);"#;

const INSERT_DAILY_SUBSTRING: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-2'), ('sk-c', 'user-3'), ('sk-d', 'user-4');
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-05', 'sk-a', CURRENT_TIMESTAMP),
 ('2', 'User-Agent:alpha', '2026-01-05', 'sk-b', CURRENT_TIMESTAMP),
 ('3', 'User-Agent:alpha-beta', '2026-01-05', 'sk-c', CURRENT_TIMESTAMP),
 ('4', 'User-Agent:beta', '2026-01-05', 'sk-d', CURRENT_TIMESTAMP);"#;

const INSERT_NULL_TAG: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-2');
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-05', 'sk-a', CURRENT_TIMESTAMP),
 ('2', NULL, '2026-01-05', 'sk-b', CURRENT_TIMESTAMP);"#;

const INSERT_WEEKLY_ROWS: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-1'), ('sk-c', 'user-2'),
 ('sk-d', 'user-3'), ('sk-e', 'user-4'), ('sk-f', 'user-2');
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-11', 'sk-a', CURRENT_TIMESTAMP),
 ('2', 'User-Agent:alpha', '2026-01-05', 'sk-b', CURRENT_TIMESTAMP),
 ('3', 'User-Agent:alpha', '2026-01-04', 'sk-c', CURRENT_TIMESTAMP),
 ('4', 'User-Agent:alpha', '2025-11-24', 'sk-d', CURRENT_TIMESTAMP),
 ('5', 'User-Agent:alpha', '2025-11-23', 'sk-e', CURRENT_TIMESTAMP),
 ('6', 'User-Agent:alpha', '2026-01-11', 'sk-f', CURRENT_TIMESTAMP);"#;

const INSERT_WEEKLY_TAGS: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, user_id) VALUES
 ('sk-a', 'user-1'), ('sk-b', 'user-2');
INSERT INTO "LiteLLM_DailyTagSpend" (id, tag, date, api_key, updated_at) VALUES
 ('1', 'User-Agent:alpha', '2026-01-05', 'sk-a', CURRENT_TIMESTAMP),
 ('2', 'User-Agent:beta', '2026-01-05', 'sk-b', CURRENT_TIMESTAMP);"#;

fn tags(body: &ActiveUsersBody) -> Vec<&str> {
    body.results.iter().map(|row| row.tag.as_str()).collect()
}

fn point(tag: &str, active_users: i64, date: &str, start: &str, end: &str) -> ActiveUserBody {
    ActiveUserBody {
        tag: tag.to_owned(),
        active_users,
        date: date.to_owned(),
        period_start: Some(start.to_owned()),
        period_end: Some(end.to_owned()),
    }
}

#[tokio::test]
async fn the_distinct_report_reads_the_pattern_by_usage_count() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_DISTINCT_ROWS).await;
    let reply = get(postgres_app(&pool), Some(&caller()), "/tag/distinct").await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: DistinctBody = reply.json();
    let found: Vec<&str> = body.results.iter().map(|row| row.tag.as_str()).collect();
    assert_eq!(
        found,
        vec!["User-Agent:alpha", "User-Agent:beta", "plain-tag"]
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_distinct_report_returns_at_most_two_hundred_and_fifty_tags() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_MANY_TAGS).await;
    let reply = get(postgres_app(&pool), Some(&caller()), "/tag/distinct").await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: DistinctBody = reply.json();
    assert_eq!(body.results.len(), 250);
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_daily_report_windows_rows_between_the_start_and_the_next_midnight() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_DAILY_WINDOW).await;
    let reply = get(postgres_app(&pool), Some(&caller()), "/tag/dau").await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ActiveUsersBody = reply.json();
    let dates: Vec<&str> = body.results.iter().map(|row| row.date.as_str()).collect();
    assert_eq!(dates, vec!["2026-01-11", "2026-01-04"]);
    assert!(body.results.iter().all(|row| row.active_users == 1));
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_daily_report_counts_distinct_joined_users() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_DAILY_USERS).await;
    let reply = get(postgres_app(&pool), Some(&caller()), "/tag/dau").await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ActiveUsersBody = reply.json();
    assert_eq!(
        body.results,
        vec![ActiveUserBody {
            tag: "User-Agent:alpha".to_owned(),
            active_users: 2,
            date: "2026-01-05".to_owned(),
            period_start: None,
            period_end: None,
        }]
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_daily_report_filters_by_an_exact_tag_list() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_DAILY_EXACT).await;
    let uri = "/tag/dau?tag_filters=User-Agent%3Aalpha&tag_filters=User-Agent%3Abeta&tag_filter=User-Agent%3Ag";
    let reply = get(postgres_app(&pool), Some(&caller()), uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ActiveUsersBody = reply.json();
    assert_eq!(tags(&body), vec!["User-Agent:alpha", "User-Agent:beta"]);
    close(fixture, pool).await;
}

#[rstest]
#[case::lowercase("/tag/dau?tag_filter=alpha")]
#[case::uppercase("/tag/dau?tag_filter=ALPHA")]
#[tokio::test]
async fn the_daily_report_filters_by_a_substring_pattern(#[case] uri: &str) {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_DAILY_SUBSTRING).await;
    let reply = get(postgres_app(&pool), Some(&caller()), uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ActiveUsersBody = reply.json();
    assert_eq!(
        tags(&body),
        vec!["User-Agent:alpha", "User-Agent:alpha-beta"]
    );
    close(fixture, pool).await;
}

#[rstest]
#[case::substring("/tag/dau?tag_filter=%27%20OR%20%271%27%3D%271")]
#[case::exact("/tag/dau?tag_filters=%27%20OR%20%271%27%3D%271")]
#[tokio::test]
async fn an_injected_filter_value_is_matched_as_text(#[case] uri: &str) {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_DAILY_EXACT).await;
    let reply = get(postgres_app(&pool), Some(&caller()), uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ActiveUsersBody = reply.json();
    assert!(body.results.is_empty());
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_blank_substring_filter_matches_every_labelled_tag() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_NULL_TAG).await;
    let reply = get(postgres_app(&pool), Some(&caller()), "/tag/dau?tag_filter=").await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ActiveUsersBody = reply.json();
    assert_eq!(tags(&body), vec!["User-Agent:alpha"]);
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_null_tag_row_fails_the_daily_report() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_NULL_TAG).await;
    let reply = get(postgres_app(&pool), Some(&caller()), "/tag/dau").await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailBody = reply.json();
    assert_eq!(
        body.detail,
        "Failed to fetch DAU analytics: database driver error"
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_weekly_report_groups_rows_into_seven_day_buckets() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_WEEKLY_ROWS).await;
    let reply = get(postgres_app(&pool), Some(&caller()), "/tag/wau").await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ActiveUsersBody = reply.json();
    assert_eq!(
        body.results,
        vec![
            point(
                "User-Agent:alpha",
                1,
                "Week 1 (Nov 24)",
                "2025-11-24",
                "2025-11-30"
            ),
            point(
                "User-Agent:alpha",
                1,
                "Week 6 (Dec 29)",
                "2025-12-29",
                "2026-01-04"
            ),
            point(
                "User-Agent:alpha",
                2,
                "Week 7 (Jan 05)",
                "2026-01-05",
                "2026-01-11"
            ),
        ]
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_weekly_report_filters_rows_inside_the_window() {
    let (fixture, pool) = database().await;
    seed(&pool, INSERT_WEEKLY_TAGS).await;
    let uri = "/tag/wau?tag_filters=User-Agent%3Aalpha";
    let reply = get(postgres_app(&pool), Some(&caller()), uri).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ActiveUsersBody = reply.json();
    assert_eq!(
        body.results,
        vec![point(
            "User-Agent:alpha",
            1,
            "Week 7 (Jan 05)",
            "2026-01-05",
            "2026-01-11"
        )]
    );
    close(fixture, pool).await;
}
