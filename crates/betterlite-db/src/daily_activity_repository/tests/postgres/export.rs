use super::super::support::scope;
use super::support::{PostgresRepository, Usage, insert, insert_all, repository, run, usage};
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityScope, ExportRow, ExportType};
use crate::daily_activity_sql::PTU_SENTINEL_API_KEY;
use rust_decimal::prelude::ToPrimitive as _;

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

#[tokio::test]
async fn postgres_four_export_types() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::Team;
        insert_all(
            &client,
            table,
            [
            Usage {
                model: Some("m1"),
                ..usage("a", Some("t1"), "2026-08-01", "k1", 1.0)
            },
            Usage {
                model: Some("m2"),
                ..usage("b", Some("t1"), "2026-08-01", "k2", 2.0)
            },
            Usage {
                model: Some("m1"),
                ..usage("c", Some("t1"), "2026-08-02", "k1", 4.0)
            },
            usage("d", Some("t2"), "2026-08-01", "k1", 8.0),
            usage("s", Some("t1"), "2026-08-01", PTU_SENTINEL_API_KEY, 0.0),
            ],
        )
        .await;
        client
            .batch_execute(
                "UPDATE \"LiteLLM_DailyTeamSpend\" SET ptu_flat_cost = 5 WHERE id = 's';
                 INSERT INTO \"LiteLLM_UserTable\" (user_id, user_email) VALUES ('u1', 'u1@example.com');
                 INSERT INTO \"LiteLLM_VerificationToken\" (token, key_alias, user_id) VALUES ('k1', 'alias1', 'u1');
                 INSERT INTO \"LiteLLM_DeletedVerificationToken\" (id, token, key_alias, user_id) VALUES
                    ('x1', 'k1', 'stale', 'u9'),
                    ('x2', 'k2', 'alias2', 'u2');",
            )
            .await
            .unwrap();
        let range = scope(table);
        let daily = collected(&repository, &range, ExportType::Daily).await;
        assert_eq!(
            rendered(&daily),
            [
                "2026-08-01|t1|-|-|-|-|-|3|5",
                "2026-08-01|t2|-|-|-|-|-|8|0",
                "2026-08-02|t1|-|-|-|-|-|4|0",
            ]
        );
        assert_eq!(daily[0].prompt_tokens, 30);
        assert_eq!(daily[0].completion_tokens, 15);
        assert_eq!(daily[0].api_requests, 3);
        let keys = collected(&repository, &range, ExportType::DailyWithKeys).await;
        assert_eq!(
            rendered(&keys),
            [
                "2026-08-01|t1|k1|-|u1|alias1|u1@example.com|1|0",
                "2026-08-01|t1|k2|-|u2|alias2|-|2|0",
                "2026-08-01|t2|k1|-|u1|alias1|u1@example.com|8|0",
                "2026-08-02|t1|k1|-|u1|alias1|u1@example.com|4|0",
            ]
        );
        let models = collected(&repository, &range, ExportType::DailyWithModels).await;
        assert_eq!(
            rendered(&models),
            [
                "2026-08-01|t1|-|m1|-|-|-|1|0",
                "2026-08-01|t1|-|m2|-|-|-|2|0",
                "2026-08-01|t2|-|-|-|-|-|8|0",
                "2026-08-02|t1|-|m1|-|-|-|4|0",
            ]
        );
        let users = collected(&repository, &range, ExportType::DailyWithUsers).await;
        assert_eq!(
            rendered(&users),
            [
                "2026-08-01|t1|-|-|u1|-|u1@example.com|1|0",
                "2026-08-01|t1|-|-|u2|-|-|2|0",
                "2026-08-01|t2|-|-|u1|-|u1@example.com|8|0",
                "2026-08-02|t1|-|-|u1|-|u1@example.com|4|0",
            ]
        );
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
