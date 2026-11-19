use std::time::Duration;

use betterlite_error::rate_limit::RateLimitCategory;
use rust_decimal::Decimal;
use tokio::time::timeout;

use crate::routing_read_batch::{RequestReads, RoutingPrefetch};
use crate::routing_value_store::{MAX_KEY_BYTES, StoreError};
use crate::usage_routing_v2::error::{
    DeploymentLoad, KeyError, LimitError, NoDeploymentAvailable, SelectionError,
};
use crate::usage_routing_v2::selection::CounterKeys;

use super::support::{
    FakeStore, TestClock, at, deployment, first, number, plain, reads, rpm_window, seed, selection,
    service, text, tiers, tpm_window, usage_tier, values,
};

pub(super) async fn run(name: &str) {
    match name {
        "no_deployment_reports_every_loaded_deployment" => {
            no_deployment_reports_every_loaded_deployment();
        }
        "the_async_facade_reports_the_same_pool" => {
            the_async_facade_reports_the_same_pool().await;
        }
        "the_error_counts_an_unread_pool_as_zero" => the_error_counts_an_unread_pool_as_zero(),
        "the_error_keeps_a_missing_value_missing" => the_error_keeps_a_missing_value_missing(),
        "the_error_carries_the_throttling_shape" => the_error_carries_the_throttling_shape(),
        "a_limit_that_is_not_a_number_names_its_field" => {
            a_limit_that_is_not_a_number_names_its_field();
        }
        "the_rpm_limit_is_named_too" => the_rpm_limit_is_named_too(),
        "a_key_past_the_store_limit_names_its_deployment" => {
            a_key_past_the_store_limit_names_its_deployment();
        }
        "an_empty_pool_has_no_candidate" => an_empty_pool_has_no_candidate(),
        "a_cancelled_read_leaves_the_request_clean" => {
            a_cancelled_read_leaves_the_request_clean().await;
        }
        other => panic!("unknown case {other}"),
    }
}

fn no_deployment_reports_every_loaded_deployment() {
    let pool = [
        deployment("a", &[("tpm", number(10)), ("rpm", number(2))], &[], &[]),
        deployment("b", &[("tpm", number(1))], &[], &[]),
    ];
    let tier = usage_tier(&TestClock::new());
    seed(
        &tier,
        &[
            (tpm_window("a").as_str(), 5),
            (tpm_window("b").as_str(), 2),
            (rpm_window("a").as_str(), 1),
        ],
    );
    let error = service(&pool, 6)
        .select_now(Some(&tier), &mut first())
        .unwrap_err();
    let SelectionError::NoDeployment(reported) = error else {
        panic!("the pool is past its limits: {error:?}");
    };
    let expected = vec![
        DeploymentLoad {
            id: "a".to_owned(),
            current_tpm: Some(5),
            tpm_limit: Some(Decimal::TEN),
            current_rpm: Some(1),
            rpm_limit: Some(Decimal::TWO),
        },
        DeploymentLoad {
            id: "b".to_owned(),
            current_tpm: Some(2),
            tpm_limit: Some(Decimal::ONE),
            current_rpm: None,
            rpm_limit: None,
        },
    ];
    assert_eq!(reported.deployments, expected);
}

// the async facade reads the same counters from its store and reports the same pool, message included
async fn the_async_facade_reports_the_same_pool() {
    let pool = [
        deployment("a", &[("tpm", number(10)), ("rpm", number(2))], &[], &[]),
        deployment("b", &[("tpm", number(1))], &[], &[]),
    ];
    let store = FakeStore::new(&[
        (tpm_window("a").as_str(), 5),
        (tpm_window("b").as_str(), 2),
        (rpm_window("a").as_str(), 1),
    ]);
    let clock = TestClock::new();
    let mut request = RequestReads::default();
    let failed = service(&pool, 6)
        .select_async(&store, &tiers(&clock), &mut request, &mut first())
        .await
        .unwrap_err();
    let SelectionError::NoDeployment(reported) = failed else {
        panic!("the async pool is past its limits: {failed:?}");
    };
    let tpm = values(&[Some(5), Some(2)]);
    let rpm = values(&[Some(1), None]);
    let expected = selection(&pool, Some(&tpm), Some(&rpm), 6)
        .no_deployment()
        .unwrap();
    assert_eq!(reported, expected);
    assert_eq!(
        reported.to_string(),
        "No deployments available for selected model. Passed model=gpt-4o. Deployments={'a': {'current_tpm': 5, 'tpm_limit': 10, 'current_rpm': 1, 'rpm_limit': 2}, 'b': {'current_tpm': 2, 'tpm_limit': 1, 'current_rpm': None, 'rpm_limit': inf}}"
    );
}

fn the_error_counts_an_unread_pool_as_zero() {
    let pool = [
        deployment("a", &[("tpm", number(0))], &[], &[]),
        deployment("b", &[("tpm", number(0))], &[], &[]),
    ];
    let error = service(&pool, 1)
        .select_now(None, &mut first())
        .unwrap_err();
    let SelectionError::NoDeployment(reported) = error else {
        panic!("both deployments are out of tokens: {error:?}");
    };
    let tpm: Vec<Option<i64>> = reported
        .deployments
        .iter()
        .map(|load| load.current_tpm)
        .collect();
    let rpm: Vec<Option<i64>> = reported
        .deployments
        .iter()
        .map(|load| load.current_rpm)
        .collect();
    assert_eq!(tpm, vec![Some(0), Some(0)]);
    assert_eq!(rpm, vec![Some(0), Some(0)]);
    assert!(
        reported
            .to_string()
            .contains("'current_tpm': 0, 'tpm_limit': 0, 'current_rpm': 0, 'rpm_limit': inf")
    );
    let empty = selection(&pool, Some(&[]), Some(&[]), 1)
        .no_deployment()
        .unwrap();
    // an empty value list is falsy in the pinned `tpm_values[index] if tpm_values else 0`
    assert!(
        empty
            .deployments
            .iter()
            .all(|load| load.current_tpm == Some(0) && load.current_rpm == Some(0))
    );
}

fn the_error_keeps_a_missing_value_missing() {
    let pool = [plain("a"), plain("b")];
    let tpm = values(&[None, Some(2)]);
    let rpm = values(&[Some(7), None]);
    let reported = selection(&pool, Some(&tpm), Some(&rpm), 0)
        .no_deployment()
        .unwrap();
    assert_eq!(reported.deployments[0].current_tpm, None);
    assert_eq!(reported.deployments[0].current_rpm, Some(7));
    assert_eq!(reported.deployments[1].current_tpm, Some(2));
    assert_eq!(reported.deployments[1].current_rpm, None);
    assert!(
        reported
            .to_string()
            .contains("'current_tpm': None, 'tpm_limit': inf, 'current_rpm': 7, 'rpm_limit': inf")
    );
}

fn the_error_carries_the_throttling_shape() {
    assert_eq!(NoDeploymentAvailable::STATUS_CODE, 429);
    assert_eq!(NoDeploymentAvailable::CODE, "429");
    assert_eq!(NoDeploymentAvailable::ERROR_TYPE, "throttling_error");
    assert_eq!(NoDeploymentAvailable::RETRY_AFTER_SECONDS, 60);
    assert_eq!(
        NoDeploymentAvailable::CATEGORY,
        RateLimitCategory::VendorRateLimit
    );
}

fn a_limit_that_is_not_a_number_names_its_field() {
    let pool = [deployment("a", &[("tpm", text("many"))], &[], &[])];
    let error = selection(&pool, None, None, 0)
        .choose(&mut first())
        .unwrap_err();
    assert_eq!(
        error,
        LimitError::NonNumeric {
            deployment: "a".to_owned(),
            field: "tpm",
        }
    );
    assert_eq!(error.field(), "tpm");
    let failed = service(&pool, 0)
        .select_now(None, &mut first())
        .unwrap_err();
    assert!(matches!(failed, SelectionError::Limit(_)));
}

fn the_rpm_limit_is_named_too() {
    let pool = [deployment("a", &[], &[("rpm", text("fast"))], &[])];
    let error = selection(&pool, None, None, 0)
        .choose(&mut first())
        .unwrap_err();
    assert_eq!(
        error,
        LimitError::NonNumeric {
            deployment: "a".to_owned(),
            field: "rpm",
        }
    );
    assert_eq!(error.field(), "rpm");
}

fn a_key_past_the_store_limit_names_its_deployment() {
    let id = "x".repeat(MAX_KEY_BYTES);
    let error = CounterKeys::for_deployments(&[plain(&id)], at(0)).unwrap_err();
    let KeyError::TooLong { deployment, source } = error;
    assert_eq!(deployment, id);
    let StoreError::KeyTooLong { bytes, limit } = source else {
        panic!("the key is past the store's limit: {source}");
    };
    assert_eq!(limit, MAX_KEY_BYTES);
    assert!(bytes > MAX_KEY_BYTES);
}

fn an_empty_pool_has_no_candidate() {
    let none = selection(&[], Some(&[]), Some(&[]), 0)
        .choose(&mut first())
        .unwrap();
    assert_eq!(none, None);
    let error = service(&[], 0).select_now(None, &mut first()).unwrap_err();
    let SelectionError::NoDeployment(reported) = error else {
        panic!("an empty pool has no deployment: {error:?}");
    };
    assert!(reported.deployments.is_empty());
    assert_eq!(
        reported.to_string(),
        "No deployments available for selected model. Passed model=gpt-4o. Deployments={}"
    );
}

async fn a_cancelled_read_leaves_the_request_clean() {
    let pool = [plain("a")];
    let mut store = FakeStore::new(&[]);
    store.hang();
    let clock = TestClock::new();
    let tiers = tiers(&clock);
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &reads(&["a"]),
        &tiers,
        true,
        &mut request
    ));
    let router = service(&pool, 0);
    let mut random = first();
    let selection = router.select_async(&store, &tiers, &mut request, &mut random);
    let cancelled = timeout(Duration::ZERO, selection).await;
    let _elapsed = cancelled.unwrap_err();
    assert!(request.prefetch.is_none());
    store.go_live();
    let picked = service(&pool, 0)
        .select_async(&store, &tiers, &mut request, &mut first())
        .await
        .unwrap();
    assert_eq!(picked, 0);
    assert_eq!(store.reads().len(), 2);
}
