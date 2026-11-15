use super::super::support::scope;
use super::support::{PostgresRepository, insert, insert_all, repository, run, run_pair, usage};
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityScope, ExportType};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use std::collections::BTreeSet;

const ROUTING_WRITER_SEED: &str = "INSERT INTO \"LiteLLM_DailyUserSpend\" (id, user_id, date, api_key, model, spend, prompt_tokens, completion_tokens, cache_read_input_tokens, api_requests, successful_requests, failed_requests, updated_at) VALUES
    ('w1', 'e1', '2026-08-01', 'k-writer', NULL, 7.0, 10, 5, 0, 1, 1, 0, NOW());
INSERT INTO \"LiteLLM_VerificationToken\" (token, key_alias) VALUES ('sk-1', 'writer')";

const ROUTING_READER_SEED: &str = "INSERT INTO \"LiteLLM_DailyUserSpend\" (id, user_id, date, api_key, model, spend, prompt_tokens, completion_tokens, cache_read_input_tokens, api_requests, successful_requests, failed_requests, updated_at) VALUES
    ('r1', 'e1', '2026-08-01', 'k-reader', NULL, 1.0, 10, 5, 0, 1, 1, 0, NOW()),
    ('r2', 'e1', '2026-08-01', 'k-reader', NULL, 1.0, 10, 5, 0, 1, 1, 0, NOW());
INSERT INTO \"LiteLLM_VerificationToken\" (token, key_alias) VALUES ('sk-1', 'reader')";

const READS_SEED: &str = "INSERT INTO \"LiteLLM_DailyTeamSpend\" (id, team_id, date, api_key, model, spend, prompt_tokens, completion_tokens, cache_read_input_tokens, api_requests, successful_requests, failed_requests, updated_at) VALUES
    ('a', 't1', '2026-08-01', 'k1', NULL, 1.0, 10, 5, 0, 1, 1, 0, NOW()),
    ('b', 't1', '2026-08-02', 'k2', NULL, 2.0, 10, 5, 0, 1, 1, 0, NOW());
INSERT INTO \"LiteLLM_VerificationToken\" (token, key_alias) VALUES ('k1', 'alias1');
INSERT INTO \"LiteLLM_DeletedVerificationToken\" (id, token, key_alias) VALUES ('x1', 'k2', 'alias2');
SET default_transaction_read_only = on";

async fn drain(
    repository: &PostgresRepository<'_>,
    range: &DailyActivityScope,
    export_type: ExportType,
) {
    let mut export = repository.export_rows(range, export_type);
    while export.next_row().await.unwrap().is_some() {}
}

#[tokio::test]
async fn postgres_count_and_rows_need_not_share_snapshot() {
    run_pair(|writer_pool, reader_pool| async move {
        let writer = writer_pool.get().await.unwrap();
        let reader = reader_pool.get().await.unwrap();
        let repository = repository(&writer, Some(&reader));
        let table = DailyActivityTable::User;
        insert(
            &writer,
            table,
            &usage("w1", Some("e1"), "2026-08-01", "k1", 1.0),
        )
        .await;
        insert_all(
            &reader,
            table,
            [
                usage("r1", Some("e1"), "2026-08-01", "k1", 1.0),
                usage("r2", Some("e1"), "2026-08-02", "k1", 1.0),
                usage("r3", Some("e1"), "2026-08-03", "k1", 1.0),
            ],
        )
        .await;
        let first = repository.daily_rows(&scope(table), 1, 2).await.unwrap();
        let first_ids: Vec<&str> = first.rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(first.total_count, 3);
        assert_eq!(first_ids, ["r3", "r2"]);
        insert(
            &reader,
            table,
            &usage("r4", Some("e1"), "2026-08-04", "k1", 1.0),
        )
        .await;
        let second = repository.daily_rows(&scope(table), 1, 2).await.unwrap();
        let second_ids: Vec<&str> = second.rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(second.total_count, 4);
        assert_eq!(second_ids, ["r4", "r3"]);
    })
    .await;
}

async fn assert_routing(repository: &PostgresRepository<'_>, range: &DailyActivityScope) {
    let aggregated = repository.aggregated(range, true, 10).await.unwrap();
    let total = aggregated
        .grouping_rows
        .iter()
        .find(|row| row.group_level == 127)
        .unwrap();
    assert_eq!(total.metrics.spend, Decimal::from_f64(7.0));
    assert_eq!(aggregated.distinct_api_keys, 1);
    let page = repository.key_page(range, 0, 10).await.unwrap();
    let ranked: Vec<&str> = page.rows.iter().map(|row| row.api_key.as_str()).collect();
    assert_eq!(ranked, ["k-writer"]);
    let found = repository.search_keys(range, "k-", 10).await.unwrap();
    assert_eq!(found, ["k-writer"]);
    let mut export = repository.export_rows(range, ExportType::Daily);
    let exported = export.next_row().await.unwrap().unwrap();
    assert_eq!(Some(exported.spend), Decimal::from_f64(7.0));
    assert!(export.next_row().await.unwrap().is_none());

    let page_rows = repository.daily_rows(range, 1, 10).await.unwrap();
    let page_ids: Vec<&str> = page_rows.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(page_rows.total_count, 2);
    assert_eq!(page_ids, ["r1", "r2"]);
    let wanted = BTreeSet::from(["sk-1".to_owned()]);
    repository.key_metadata(&wanted, None).await.unwrap();
    let call = repository.proxy_reads.last_call();
    assert_eq!(call.resolved["sk-1"].key_alias.as_deref(), Some("reader"));
}

#[tokio::test]
async fn raw_writer_table_reader_routing() {
    run_pair(|writer_pool, reader_pool| async move {
        let writer = writer_pool.get().await.unwrap();
        let reader = reader_pool.get().await.unwrap();
        let repository = repository(&writer, Some(&reader));
        writer.batch_execute(ROUTING_WRITER_SEED).await.unwrap();
        reader.batch_execute(ROUTING_READER_SEED).await.unwrap();
        assert_routing(&repository, &scope(DailyActivityTable::User)).await;
    })
    .await;
}

#[tokio::test]
async fn postgres_reads_never_write() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::Team;
        client.batch_execute(READS_SEED).await.unwrap();
        let range = scope(table);
        repository.aggregated(&range, true, 10).await.unwrap();
        repository.key_page(&range, 0, 10).await.unwrap();
        repository.daily_rows(&range, 1, 10).await.unwrap();
        repository.search_keys(&range, "alias", 10).await.unwrap();
        repository
            .model_top_keys(&range, "m1", false, 10)
            .await
            .unwrap();
        repository.cache_leakage_keys(&range, 10).await.unwrap();
        let wanted = BTreeSet::from(["k1".to_owned(), "k2".to_owned()]);
        repository.key_metadata(&wanted, None).await.unwrap();
        drain(&repository, &range, ExportType::Daily).await;
        drain(&repository, &range, ExportType::DailyWithKeys).await;
        drain(&repository, &range, ExportType::DailyWithModels).await;
        drain(&repository, &range, ExportType::DailyWithUsers).await;
        let rejected = client
            .batch_execute("DELETE FROM \"LiteLLM_DailyTeamSpend\"")
            .await;
        rejected.unwrap_err();
        let count = client
            .query_one(
                "SELECT COUNT(*)::bigint FROM \"LiteLLM_DailyTeamSpend\"",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(count.get::<_, i64>(0), 2);
    })
    .await;
}
