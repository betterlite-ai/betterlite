use super::super::support::scope;
use super::support::{PostgresRepository, insert, repository, run, usage};
use crate::daily_activity_contracts::ExportRow;
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityScope, ExportType};
use rust_decimal::prelude::ToPrimitive as _;

const EXPORT_SEED: &str = "INSERT INTO \"LiteLLM_DailyTeamSpend\" (id, team_id, date, api_key, model, spend, prompt_tokens, completion_tokens, cache_read_input_tokens, api_requests, successful_requests, failed_requests, updated_at) VALUES
    ('a', 't1', '2026-08-01', 'k1', 'm1', 1.0, 10, 5, 0, 1, 1, 0, NOW()),
    ('b', 't1', '2026-08-01', 'k2', 'm2', 2.0, 10, 5, 0, 1, 1, 0, NOW()),
    ('c', 't1', '2026-08-02', 'k1', 'm1', 4.0, 10, 5, 0, 1, 1, 0, NOW()),
    ('d', 't2', '2026-08-01', 'k1', NULL, 8.0, 10, 5, 0, 1, 1, 0, NOW()),
    ('s', 't1', '2026-08-01', '__ptu_flat_cost__', NULL, 0.0, 10, 5, 0, 1, 1, 0, NOW());
UPDATE \"LiteLLM_DailyTeamSpend\" SET ptu_flat_cost = 5 WHERE id = 's';
INSERT INTO \"LiteLLM_UserTable\" (user_id, user_email) VALUES ('u1', 'u1@example.com');
INSERT INTO \"LiteLLM_VerificationToken\" (token, key_alias, user_id) VALUES ('k1', 'alias1', 'u1');
INSERT INTO \"LiteLLM_DeletedVerificationToken\" (id, token, key_alias, user_id) VALUES
    ('x1', 'k1', 'stale', 'u9'),
    ('x2', 'k2', 'alias2', 'u2')";

const DAILY_EXPORT: [&str; 3] = [
    "2026-08-01|t1|-|-|-|-|-|3|5",
    "2026-08-01|t2|-|-|-|-|-|8|0",
    "2026-08-02|t1|-|-|-|-|-|4|0",
];

const KEY_EXPORT: [&str; 4] = [
    "2026-08-01|t1|k1|-|u1|alias1|u1@example.com|1|0",
    "2026-08-01|t1|k2|-|u2|alias2|-|2|0",
    "2026-08-01|t2|k1|-|u1|alias1|u1@example.com|8|0",
    "2026-08-02|t1|k1|-|u1|alias1|u1@example.com|4|0",
];

const MODEL_EXPORT: [&str; 4] = [
    "2026-08-01|t1|-|m1|-|-|-|1|0",
    "2026-08-01|t1|-|m2|-|-|-|2|0",
    "2026-08-01|t2|-|-|-|-|-|8|0",
    "2026-08-02|t1|-|m1|-|-|-|4|0",
];

const USER_EXPORT: [&str; 4] = [
    "2026-08-01|t1|-|-|u1|-|u1@example.com|1|0",
    "2026-08-01|t1|-|-|u2|-|-|2|0",
    "2026-08-01|t2|-|-|u1|-|u1@example.com|8|0",
    "2026-08-02|t1|-|-|u1|-|u1@example.com|4|0",
];

async fn collected(
    repository: &PostgresRepository<'_>,
    range: &DailyActivityScope,
    export_type: ExportType,
) -> Vec<ExportRow> {
    let mut export = repository.export_rows(range, export_type);
    let mut rows = Vec::new();
    while let Some(row) = export.next_row().await.unwrap() {
        rows.push(row);
    }
    rows
}

fn rendered(rows: &[ExportRow]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}",
                row.date,
                row.entity_id,
                row.api_key.as_deref().unwrap_or("-"),
                row.model.as_deref().unwrap_or("-"),
                row.user_id.as_deref().unwrap_or("-"),
                row.key_alias.as_deref().unwrap_or("-"),
                row.user_email.as_deref().unwrap_or("-"),
                row.spend.to_f64().unwrap(),
                row.flat_cost.to_f64().unwrap(),
            )
        })
        .collect()
}

async fn assert_export(
    repository: &PostgresRepository<'_>,
    range: &DailyActivityScope,
    export_type: ExportType,
    expected: &[&str],
) -> Vec<ExportRow> {
    let rows = collected(repository, range, export_type).await;
    assert_eq!(rendered(&rows), expected);
    rows
}

#[tokio::test]
async fn postgres_four_export_types() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::Team;
        client.batch_execute(EXPORT_SEED).await.unwrap();
        let range = scope(table);
        let daily = assert_export(&repository, &range, ExportType::Daily, &DAILY_EXPORT).await;
        assert_eq!(daily[0].prompt_tokens, 30);
        assert_eq!(daily[0].completion_tokens, 15);
        assert_eq!(daily[0].api_requests, 3);
        assert_export(&repository, &range, ExportType::DailyWithKeys, &KEY_EXPORT).await;
        assert_export(
            &repository,
            &range,
            ExportType::DailyWithModels,
            &MODEL_EXPORT,
        )
        .await;
        assert_export(
            &repository,
            &range,
            ExportType::DailyWithUsers,
            &USER_EXPORT,
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn postgres_export_sees_later_commits() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::User;
        client
            .batch_execute(
                "INSERT INTO \"LiteLLM_DailyUserSpend\" (id, user_id, date, api_key, model, spend, prompt_tokens, completion_tokens, cache_read_input_tokens, api_requests, successful_requests, failed_requests, updated_at)
                 SELECT 'bulk-' || n::text, 'u' || LPAD(n::text, 4, '0'), '2026-08-01', 'k1', NULL, 1.0, 1, 1, 0, 1, 1, 0, NOW()
                 FROM generate_series(1, 1000) AS n",
            )
            .await
            .unwrap();
        let range = scope(table);
        let mut export = repository.export_rows(&range, ExportType::Daily);
        let first = export.next_row().await.unwrap().unwrap();
        assert_eq!(first.entity_id, "u0001");
        insert(
            &client,
            table,
            &usage("early", Some("u0000"), "2026-08-01", "k1", 1.0),
        )
        .await;
        insert(
            &client,
            table,
            &usage("late", Some("u2000"), "2026-08-02", "k1", 1.0),
        )
        .await;
        let mut rest = Vec::new();
        while let Some(row) = export.next_row().await.unwrap() {
            rest.push(row);
        }
        assert_eq!(rest.len(), 1000);
        assert_eq!(rest[998].entity_id, "u1000");
        assert_eq!(rest[999].entity_id, "u2000");
        assert_eq!(rest[999].date, "2026-08-02");
        assert!(rest.iter().all(|row| row.entity_id != "u0000"));
    })
    .await;
}
