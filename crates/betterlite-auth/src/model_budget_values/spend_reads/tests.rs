#![cfg(test)]
use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use super::error::SpendReadError;
use super::{CachedSpend, current_window_spends};
use crate::model_budget_values::tests::support::{FakeCache, FakeFailure, amount};

fn keys(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn redis_with(
    entries: &[(&str, CachedSpend)],
    memory: Option<Vec<Option<CachedSpend>>>,
) -> FakeCache {
    let held: BTreeMap<String, CachedSpend> = entries
        .iter()
        .map(|(key, spend)| ((*key).to_owned(), spend.clone()))
        .collect();
    FakeCache::new(Some(Ok(held)), memory)
}

#[tokio::test]
async fn spend_reads_prefer_redis_and_never_touch_memory() {
    let cache = redis_with(
        &[("a", CachedSpend::Number(amount(15, 1)))],
        Some(vec![Some(CachedSpend::Number(amount(99, 0)))]),
    );
    let spends = current_window_spends(&cache, &keys(&["a"])).await.unwrap();
    assert_eq!(spends, [amount(15, 1)]);
    assert_eq!(cache.redis_reads.lock().unwrap().clone(), [["a"]]);
    assert!(cache.memory_reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn spend_reads_count_a_key_redis_lacks_as_zero_without_asking_memory() {
    let cache = redis_with(
        &[("b", CachedSpend::Number(amount(2, 0)))],
        Some(vec![Some(CachedSpend::Number(amount(99, 0))); 2]),
    );
    let spends = current_window_spends(&cache, &keys(&["a", "b"]))
        .await
        .unwrap();
    assert_eq!(spends, [Decimal::ZERO, amount(2, 0)]);
    assert!(cache.memory_reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn spend_reads_keep_the_order_of_the_requested_keys() {
    let cache = redis_with(
        &[
            ("a", CachedSpend::Number(amount(1, 0))),
            ("b", CachedSpend::Number(amount(2, 0))),
            ("c", CachedSpend::Number(amount(3, 0))),
        ],
        None,
    );
    let spends = current_window_spends(&cache, &keys(&["c", "a", "b"]))
        .await
        .unwrap();
    assert_eq!(spends, [amount(3, 0), amount(1, 0), amount(2, 0)]);
}

#[rstest]
#[case::plain_text(CachedSpend::Text("1.25".to_owned()), amount(125, 2))]
#[case::padded_text(CachedSpend::Text(" 3 ".to_owned()), amount(3, 0))]
#[case::scientific_text(CachedSpend::Text("1e-3".to_owned()), amount(1, 3))]
#[case::garbage_text(CachedSpend::Text("lots".to_owned()), Decimal::ZERO)]
#[case::empty_text(CachedSpend::Text(String::new()), Decimal::ZERO)]
#[case::nan_text(CachedSpend::Text("nan".to_owned()), Decimal::ZERO)]
#[case::infinite_text(CachedSpend::Text("-inf".to_owned()), Decimal::ZERO)]
#[case::unreadable(CachedSpend::Unreadable, Decimal::ZERO)]
#[case::number(CachedSpend::Number(amount(-4, 1)), amount(-4, 1))]
#[tokio::test]
async fn spend_reads_turn_each_redis_value_into_an_amount(
    #[case] value: CachedSpend,
    #[case] expected: Decimal,
) {
    let cache = redis_with(&[("a", value)], None);
    let spends = current_window_spends(&cache, &keys(&["a"])).await.unwrap();
    assert_eq!(spends, [expected]);
}

#[rstest]
#[case::number(Some(CachedSpend::Number(amount(7, 1))), amount(7, 1))]
#[case::text(Some(CachedSpend::Text("2.5".to_owned())), amount(25, 1))]
#[case::garbage_text(Some(CachedSpend::Text("lots".to_owned())), Decimal::ZERO)]
#[case::unreadable(Some(CachedSpend::Unreadable), Decimal::ZERO)]
#[case::missing(None, Decimal::ZERO)]
#[tokio::test]
async fn spend_reads_fall_back_to_memory_without_redis(
    #[case] value: Option<CachedSpend>,
    #[case] expected: Decimal,
) {
    let cache = FakeCache::new(None, Some(vec![value]));
    let spends = current_window_spends(&cache, &keys(&["a"])).await.unwrap();
    assert_eq!(spends, [expected]);
    assert_eq!(cache.redis_reads.lock().unwrap().clone(), [["a"]]);
    assert_eq!(cache.memory_reads.lock().unwrap().clone(), [["a"]]);
}

#[rstest]
#[case::failed_read(None)]
#[case::too_short(Some(vec![Some(CachedSpend::Number(amount(5, 0)))]))]
#[case::too_long(Some(vec![Some(CachedSpend::Number(amount(5, 0))); 3]))]
#[case::empty_list(Some(Vec::new()))]
#[tokio::test]
async fn spend_reads_treat_an_unusable_memory_read_as_all_zero(
    #[case] memory: Option<Vec<Option<CachedSpend>>>,
) {
    let cache = FakeCache::new(None, memory);
    let spends = current_window_spends(&cache, &keys(&["a", "b"]))
        .await
        .unwrap();
    assert_eq!(spends, [Decimal::ZERO, Decimal::ZERO]);
}

#[tokio::test]
async fn spend_reads_accept_a_memory_list_of_exactly_the_requested_length() {
    let cache = FakeCache::new(
        None,
        Some(vec![
            Some(CachedSpend::Number(amount(1, 0))),
            Some(CachedSpend::Number(amount(2, 0))),
        ]),
    );
    let spends = current_window_spends(&cache, &keys(&["a", "b"]))
        .await
        .unwrap();
    assert_eq!(spends, [amount(1, 0), amount(2, 0)]);
}

#[tokio::test]
async fn spend_reads_return_nothing_for_no_keys() {
    let cache = FakeCache::new(None, Some(Vec::new()));
    assert_eq!(current_window_spends(&cache, &[]).await.unwrap(), []);
}

#[tokio::test]
async fn spend_reads_surface_a_redis_failure_and_skip_memory() {
    let cache = FakeCache::new(
        Some(Err(FakeFailure)),
        Some(vec![Some(CachedSpend::Number(amount(1, 0)))]),
    );
    let error = current_window_spends(&cache, &keys(&["a"]))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SpendReadError::Redis {
            source: FakeFailure
        }
    ));
    assert!(cache.memory_reads.lock().unwrap().is_empty());
}
