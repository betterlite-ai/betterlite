use super::super::support::scope;
use super::support::{
    PostgresRepository, Usage, insert, insert_all, repository, run, scope_with, texts, usage,
};
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityScope, GroupingSetsRow};
use crate::daily_activity_sql::PTU_SENTINEL_API_KEY;
use deadpool_postgres::Client;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

fn rendered(rows: &[GroupingSetsRow], levels: &[i64]) -> Vec<String> {
    let mut lines: Vec<String> = rows
        .iter()
        .filter(|row| levels.contains(&row.group_level))
        .map(|row| {
            format!(
                "{}|{}|{}|{}|{}|{}",
                row.group_level,
                row.metrics.date.as_deref().unwrap_or("-"),
                row.metrics.api_key.as_deref().unwrap_or("-"),
                row.model.as_deref().unwrap_or("-"),
                row.metrics.spend.unwrap().to_f64().unwrap(),
                row.metrics.prompt_tokens.unwrap(),
            )
        })
        .collect();
    lines.sort();
    lines
}

async fn check_table(
    client: &Client,
    repository: &PostgresRepository<'_>,
    table: DailyActivityTable,
) {
    insert(
        client,
        table,
        &usage("a", Some("e1"), "2026-08-01", "k1", 1.5),
    )
    .await;
    insert(
        client,
        table,
        &usage("b", Some("e2"), "2026-08-02", "k2", 2.5),
    )
    .await;
    let relation = table.metadata().relation;
    let counted = texts(
        client,
        &format!("SELECT COUNT(*)::text FROM \"{relation}\""),
    )
    .await;
    assert_eq!(counted, ["2"], "{relation}");
    let page = repository.daily_rows(&scope(table), 1, 10).await.unwrap();
    let ids: Vec<&str> = page.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(page.total_count, 2, "{relation}");
    assert_eq!(ids, ["b", "a"], "{relation}");
    let aggregated = repository
        .aggregated(&scope(table), true, 10)
        .await
        .unwrap();
    let total = aggregated
        .grouping_rows
        .iter()
        .find(|row| row.group_level == 127)
        .unwrap();
    assert_eq!(total.metrics.spend, Decimal::from_f64(4.0), "{relation}");
    assert_eq!(aggregated.distinct_api_keys, 2, "{relation}");
    assert!(aggregated.entity_rows.is_some(), "{relation}");
    let ranked = repository.key_page(&scope(table), 0, 10).await.unwrap();
    assert_eq!(ranked.total_api_keys, 2, "{relation}");
}

#[tokio::test]
async fn postgres_all_six_daily_tables() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        check_table(&client, &repository, DailyActivityTable::User).await;
        check_table(&client, &repository, DailyActivityTable::Team).await;
        check_table(&client, &repository, DailyActivityTable::Tag).await;
        check_table(&client, &repository, DailyActivityTable::Organization).await;
        check_table(&client, &repository, DailyActivityTable::Customer).await;
        check_table(&client, &repository, DailyActivityTable::Agent).await;
    })
    .await;
}

#[tokio::test]
async fn postgres_grouping_metrics_and_masks() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::User;
        insert_all(
            &client,
            table,
            [
                Usage {
                    model: Some("m1"),
                    ..usage("a", Some("e1"), "2026-08-01", "k1", 1.5)
                },
                Usage {
                    model: Some("m2"),
                    ..usage("b", Some("e1"), "2026-08-01", "k2", 2.0)
                },
                Usage {
                    model: Some("m1"),
                    ..usage("c", Some("e2"), "2026-08-02", "k1", 0.25)
                },
            ],
        )
        .await;
        let aggregated = repository
            .aggregated(&scope(table), false, 10)
            .await
            .unwrap();
        let lines = rendered(&aggregated.grouping_rows, &[127, 63, 47, 31, 15]);
        assert_eq!(
            lines,
            [
                "127|-|-|-|3.75|30",
                "15|2026-08-01|k1|m1|1.5|10",
                "15|2026-08-01|k2|m2|2|10",
                "15|2026-08-02|k1|m1|0.25|10",
                "31|2026-08-01|k1|-|1.5|10",
                "31|2026-08-01|k2|-|2|10",
                "31|2026-08-02|k1|-|0.25|10",
                "47|2026-08-01|-|m1|1.5|10",
                "47|2026-08-01|-|m2|2|10",
                "47|2026-08-02|-|m1|0.25|10",
                "63|2026-08-01|-|-|3.5|20",
                "63|2026-08-02|-|-|0.25|10",
            ]
        );
        let total = aggregated
            .grouping_rows
            .iter()
            .find(|row| row.group_level == 127)
            .unwrap();
        assert_eq!(total.metrics.api_requests, Some(3));
        assert_eq!(total.metrics.completion_tokens, Some(15));
        assert_eq!(aggregated.distinct_api_keys, 2);
        assert!(aggregated.entity_rows.is_none());
    })
    .await;
}

#[tokio::test]
async fn postgres_ptu_totals_and_global_top_keys() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::Team;
        insert_all(
            &client,
            table,
            [
                usage("a", Some("t1"), "2026-08-01", "k1", 3.0),
                usage("b", Some("t1"), "2026-08-02", "k2", 1.0),
                usage("c", Some("t2"), "2026-08-02", "k3", 0.5),
                usage("d", Some("t1"), "2026-08-02", PTU_SENTINEL_API_KEY, 0.0),
            ],
        )
        .await;
        client
            .execute(
                "UPDATE \"LiteLLM_DailyTeamSpend\" SET ptu_flat_cost = 5 WHERE id = 'd'",
                &[],
            )
            .await
            .unwrap();
        let aggregated = repository
            .aggregated(&scope(table), false, 2)
            .await
            .unwrap();
        let total = aggregated
            .grouping_rows
            .iter()
            .find(|row| row.group_level == 127)
            .unwrap();
        assert_eq!(total.metrics.ptu_flat_cost, Decimal::from_f64(5.0));
        assert_eq!(total.metrics.spend, Decimal::from_f64(4.5));
        assert_eq!(aggregated.distinct_api_keys, 3);
        let keys: BTreeSet<&str> = aggregated
            .grouping_rows
            .iter()
            .filter_map(|row| row.metrics.api_key.as_deref())
            .collect();
        assert_eq!(keys, BTreeSet::from(["k1", "k2"]));
        let page = repository.key_page(&scope(table), 0, 10).await.unwrap();
        let ranked: Vec<&str> = page.rows.iter().map(|row| row.api_key.as_str()).collect();
        assert_eq!(page.total_api_keys, 3);
        assert_eq!(ranked, ["k1", "k2", "k3"]);
    })
    .await;
}

#[tokio::test]
async fn postgres_entity_counts_and_null_groups() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::User;
        insert_all(
            &client,
            table,
            [
            usage("a", None, "2026-08-01", "k1", 1.0),
            usage("b", Some("u1"), "2026-08-01", "k1", 1.0),
            usage("c", Some("u1"), "2026-08-02", "k2", 1.0),
            usage("d", Some("u1"), "2026-08-02", "k1", 1.0),
            ],
        )
        .await;
        let aggregated = repository
            .aggregated(&scope(table), true, 10)
            .await
            .unwrap();
        let found: BTreeMap<String, i64> = aggregated
            .entity_rows
            .unwrap()
            .into_iter()
            .filter(|row| row.api_key_rolled == 1)
            .map(|row| (row.entity_id.unwrap(), row.distinct_api_keys.unwrap()))
            .collect();
        let counted = client
            .query(
                "SELECT COALESCE(user_id, '') AS entity, COUNT(DISTINCT api_key)::bigint AS keys FROM \"LiteLLM_DailyUserSpend\" GROUP BY 1",
                &[],
            )
            .await
            .unwrap();
        let expected: BTreeMap<String, i64> = counted
            .iter()
            .map(|row| (row.get("entity"), row.get("keys")))
            .collect();
        assert_eq!(found, expected);
        assert_eq!(
            found,
            BTreeMap::from([(String::new(), 1), ("u1".to_owned(), 2)])
        );
    })
    .await;
}

async fn assert_page(
    repository: &PostgresRepository<'_>,
    table: DailyActivityTable,
    ranked: &[String],
    offset: i64,
    limit: i64,
    range: Range<usize>,
) {
    let page = repository
        .key_page(&scope(table), offset, limit)
        .await
        .unwrap();
    let keys: Vec<String> = page.rows.iter().map(|row| row.api_key.clone()).collect();
    assert_eq!(page.total_api_keys, 3, "offset {offset}");
    assert_eq!(keys, ranked[range], "offset {offset}");
}

#[tokio::test]
async fn postgres_key_page_empty_and_past_end() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::User;
        let empty = repository.key_page(&scope(table), 0, 10).await.unwrap();
        assert_eq!(empty.total_api_keys, 0);
        assert!(empty.rows.is_empty());
        insert_all(
            &client,
            table,
            [
            usage("a", None, "2026-08-01", "k1", 3.0),
            usage("b", None, "2026-08-01", "k2", 1.0),
            usage("c", None, "2026-08-01", "k3", 2.0),
            ],
        )
        .await;
        let ranked = texts(
            &client,
            "SELECT api_key FROM \"LiteLLM_DailyUserSpend\" GROUP BY api_key ORDER BY SUM(spend) DESC, api_key",
        )
        .await;
        assert_eq!(ranked, ["k1", "k3", "k2"]);
        assert_page(&repository, table, &ranked, 0, 2, 0..2).await;
        assert_page(&repository, table, &ranked, 2, 2, 2..3).await;
        assert_page(&repository, table, &ranked, 10, 2, 3..3).await;
    })
    .await;
}

async fn assert_search(
    repository: &PostgresRepository<'_>,
    table: DailyActivityTable,
    search: &str,
    expected: &[&str],
) {
    let found = repository
        .search_keys(&scope(table), search, 10)
        .await
        .unwrap();
    assert_eq!(found, expected, "search {search}");
}

#[tokio::test]
async fn postgres_literal_search_active_deleted() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::User;
        insert_all(
            &client,
            table,
            [
                usage("sk-plain", None, "2026-08-01", "sk-plain", 1.0),
                usage("sk-active", None, "2026-08-01", "sk-active", 1.0),
                usage("sk-deleted", None, "2026-08-01", "sk-deleted", 1.0),
                usage("sk-user", None, "2026-08-01", "sk-user", 1.0),
                usage("sk-wild", None, "2026-08-01", "sk-wild", 1.0),
            ],
        )
        .await;
        client
            .batch_execute(
                "INSERT INTO \"LiteLLM_UserTable\" (user_id, user_email) VALUES ('u-search', 'dev@example.com');
                 INSERT INTO \"LiteLLM_VerificationToken\" (token, key_alias, user_id) VALUES ('sk-active', 'My_Alias%', NULL), ('sk-wild', 'wildXalias', NULL), ('sk-user', NULL, 'u-search');
                 INSERT INTO \"LiteLLM_DeletedVerificationToken\" (id, token, key_alias) VALUES ('d1', 'sk-deleted', 'gone_alias');",
            )
            .await
            .unwrap();
        assert_search(&repository, table, "_alias", &["sk-active", "sk-deleted"]).await;
        assert_search(&repository, table, "%", &["sk-active"]).await;
        assert_search(&repository, table, "dev@example", &["sk-user"]).await;
        assert_search(&repository, table, "PLAIN", &["sk-plain"]).await;
        let limited = repository
            .search_keys(&scope(table), "sk-", 2)
            .await
            .unwrap();
        assert_eq!(limited.len(), 2);
    })
    .await;
}

#[tokio::test]
async fn postgres_model_and_leakage_ranking() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::User;
        insert_all(
            &client,
            table,
            [
            Usage {
                model: Some("m1"),
                prompt: 100,
                cache_read: 90,
                ..usage("a", None, "2026-08-01", "k1", 1.0)
            },
            Usage {
                model: Some("m1"),
                prompt: 50,
                ..usage("b", None, "2026-08-01", "k2", 3.0)
            },
            Usage {
                model: Some("m2"),
                prompt: 70,
                cache_read: 10,
                ..usage("c", None, "2026-08-01", "k3", 2.0)
            },
            Usage {
                model: Some("m2"),
                prompt: 5,
                cache_read: 5,
                ..usage("d", None, "2026-08-01", "k4", 9.0)
            },
            ],
        )
        .await;
        client
            .execute(
                "UPDATE \"LiteLLM_DailyUserSpend\" SET model_group = 'grp' WHERE id = 'c'",
                &[],
            )
            .await
            .unwrap();
        let scope = scope(table);
        let by_model = repository
            .model_top_keys(&scope, "m1", false, 10)
            .await
            .unwrap();
        let keys: Vec<&str> = by_model.iter().map(|row| row.api_key.as_str()).collect();
        assert_eq!(keys, ["k2", "k1"]);
        assert_eq!(by_model[0].spend, Decimal::from_f64(3.0).unwrap());
        let by_group = repository
            .model_top_keys(&scope, "grp", true, 10)
            .await
            .unwrap();
        let keys: Vec<&str> = by_group.iter().map(|row| row.api_key.as_str()).collect();
        assert_eq!(keys, ["k3"]);
        let group_name_as_model = repository
            .model_top_keys(&scope, "grp", false, 10)
            .await
            .unwrap();
        assert!(group_name_as_model.is_empty());
        let truncated = repository
            .model_top_keys(&scope, "m1", false, 1)
            .await
            .unwrap();
        assert_eq!(truncated.len(), 1);
        let leakage = repository.cache_leakage_keys(&scope, 10).await.unwrap();
        let ranked = texts(
            &client,
            "SELECT api_key FROM \"LiteLLM_DailyUserSpend\" GROUP BY api_key HAVING SUM(prompt_tokens) - SUM(cache_read_input_tokens) > 0 ORDER BY SUM(prompt_tokens) - SUM(cache_read_input_tokens) DESC, api_key",
        )
        .await;
        let keys: Vec<String> = leakage.iter().map(|row| row.api_key.clone()).collect();
        assert_eq!(keys, ranked);
        assert_eq!(keys, ["k3", "k2", "k1"]);
    })
    .await;
}

async fn assert_filter(
    repository: &PostgresRepository<'_>,
    name: &str,
    scope: &DailyActivityScope,
    expected: &[&str],
) {
    let page = repository.daily_rows(scope, 1, 10).await.unwrap();
    let ids: Vec<&str> = page.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, expected, "{name}");
    assert_eq!(
        page.total_count,
        i64::try_from(expected.len()).unwrap(),
        "{name}"
    );
}

#[tokio::test]
async fn postgres_daily_row_filters_and_order() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        let table = DailyActivityTable::User;
        insert_all(
            &client,
            table,
            [
                Usage {
                    model: Some("m1"),
                    ..usage("a", Some("u1"), "2026-08-01", "k1", 1.0)
                },
                Usage {
                    model: Some("m2"),
                    ..usage("b", Some("u2"), "2026-08-03", "k2", 1.0)
                },
                Usage {
                    model: Some("m1"),
                    ..usage("c", None, "2026-08-03", "k1", 1.0)
                },
                Usage {
                    model: Some("m1"),
                    ..usage("d", Some("u1"), "2026-08-05", "k3", 1.0)
                },
                Usage {
                    model: Some("m1"),
                    ..usage("e", Some("u1"), "2026-07-31", "k1", 1.0)
                },
                Usage {
                    model: Some("m1"),
                    ..usage("f", Some("u1"), "2026-08-06", "k1", 1.0)
                },
            ],
        )
        .await;
        let second = repository.daily_rows(&scope(table), 2, 3).await.unwrap();
        assert_eq!(second.total_count, 4);
        assert_eq!(second.rows.len(), 1);
        assert_eq!(second.rows[0].id, "a");
        assert_filter(
            &repository,
            "no filter",
            &scope(table),
            &["d", "b", "c", "a"],
        )
        .await;
        assert_filter(
            &repository,
            "entity_ids",
            &scope_with(table, |input| {
                input.entity_ids = Some(vec!["u1".to_owned()])
            }),
            &["d", "a"],
        )
        .await;
        assert_filter(
            &repository,
            "empty entity_ids",
            &scope_with(table, |input| input.entity_ids = Some(Vec::new())),
            &[],
        )
        .await;
        assert_filter(
            &repository,
            "excluded keeps null",
            &scope_with(table, |input| {
                input.exclude_entity_ids = vec!["u1".to_owned()];
            }),
            &["b", "c"],
        )
        .await;
        assert_filter(
            &repository,
            "model",
            &scope_with(table, |input| input.model = Some("m2".to_owned())),
            &["b"],
        )
        .await;
        assert_filter(
            &repository,
            "empty model ignored",
            &scope_with(table, |input| input.model = Some(String::new())),
            &["d", "b", "c", "a"],
        )
        .await;
        assert_filter(
            &repository,
            "api_keys",
            &scope_with(table, |input| input.api_keys = Some(vec!["k1".to_owned()])),
            &["c", "a"],
        )
        .await;
        assert_filter(
            &repository,
            "empty api_keys",
            &scope_with(table, |input| input.api_keys = Some(Vec::new())),
            &[],
        )
        .await;
    })
    .await;
}
