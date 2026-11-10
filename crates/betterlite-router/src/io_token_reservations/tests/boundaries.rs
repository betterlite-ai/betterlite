use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use tokio::time::timeout;

use super::support::{self, Behavior, RecordingCounter};
use crate::io_token_reservations::clock::{minute_label, seconds_until_minute_reset};
use crate::io_token_reservations::error::IoPreCallError;
use crate::io_token_reservations::limits::IoDeployment;
use crate::io_token_reservations::reservation::{
    ITPM_CACHE_KEY, ITPM_RESERVED_KEY, IoReservation, OTPM_CACHE_KEY, OTPM_RESERVED_KEY,
    clear_reservation, read_reservation, stash_reservation,
};
use crate::io_token_reservations::usage::{UsageTokens, resolve_reconcile_usage};

pub(super) async fn run(name: &str) {
    if limit_cases(name).await {
        return;
    }
    if cap_cases(name).await {
        return;
    }
    if key_cases(name).await {
        return;
    }
    if metadata_cases(name) {
        return;
    }
    if usage_cases(name) {
        return;
    }
    if cancellation_cases(name).await {
        return;
    }
    panic!("unknown case {name}");
}

async fn limit_cases(name: &str) -> bool {
    match name {
        "current_equal_to_limit_passes" => current_equal_to_limit_passes().await,
        "current_over_limit_by_one_fails" => current_over_limit_by_one_fails().await,
        "zero_limit_one_token_fails" => zero_limit_one_token_fails().await,
        "negative_limit_always_fails" => negative_limit_always_fails().await,
        "negative_max_tokens_reserves_nothing" => negative_max_tokens_reserves_nothing().await,
        "model_cap_equal_to_limit_passes" => model_cap_equal_to_limit_passes().await,
        "model_cap_over_limit_by_one_fails" => model_cap_over_limit_by_one_fails().await,
        _ => return false,
    }
    true
}

async fn cap_cases(name: &str) -> bool {
    match name {
        "float_max_tokens_is_an_error" => float_max_tokens_is_an_error().await,
        "text_max_tokens_is_an_error" => text_max_tokens_is_an_error().await,
        "bool_max_tokens_is_an_error" => bool_max_tokens_is_an_error().await,
        "null_max_tokens_uses_next_field" => null_max_tokens_uses_next_field().await,
        _ => return false,
    }
    true
}

async fn key_cases(name: &str) -> bool {
    match name {
        "missing_model_id_skips_limits" => missing_model_id_skips_limits().await,
        "missing_model_skips_limits" => missing_model_skips_limits().await,
        "no_limits_skips_everything" => no_limits_skips_everything().await,
        "empty_model_id_still_builds_keys" => empty_model_id_still_builds_keys().await,
        "minute_labels" => minute_labels(),
        _ => return false,
    }
    true
}

fn metadata_cases(name: &str) -> bool {
    match name {
        "reservation_channel_priority" => reservation_channel_priority(),
        "channel_without_itpm_sentinel_is_skipped" => channel_without_itpm_sentinel_is_skipped(),
        "clear_removes_forged_sentinels" => clear_removes_forged_sentinels(),
        "stash_merges_existing_metadata" => stash_merges_existing_metadata(),
        "stash_replaces_non_object_metadata" => stash_replaces_non_object_metadata(),
        _ => return false,
    }
    true
}

fn usage_cases(name: &str) -> bool {
    match name {
        "billable_input_never_negative" => billable_input_never_negative(),
        "standard_logging_zero_tokens_unresolved" => standard_logging_zero_tokens_unresolved(),
        _ => return false,
    }
    true
}

async fn cancellation_cases(name: &str) -> bool {
    match name {
        "cancelled_output_increment_keeps_input" => cancelled_output_increment_keeps_input().await,
        _ => return false,
    }
    true
}

async fn current_equal_to_limit_passes() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let deployment = support::deployment(Some(100), None);
    let keys = support::keys(&deployment);
    counter.seed(&keys.itpm, 90);
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let mut request = support::body();
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reserved.itpm_reserved, 10);
    assert_eq!(counter.value(&keys.itpm), 100);
}

async fn current_over_limit_by_one_fails() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let deployment = support::deployment(Some(100), None);
    let keys = support::keys(&deployment);
    counter.seed(&keys.itpm, 91);
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let mut request = support::body();
    let error = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap_err();
    match error {
        IoPreCallError::RateLimit(limit) => assert_eq!(limit.current, 101),
        IoPreCallError::OutputCap { field } => panic!("unexpected cap error for {field}"),
    }
    assert_eq!(counter.value(&keys.itpm), 91);
}

async fn zero_limit_one_token_fails() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let deployment = support::deployment(Some(0), None);
    let reservations = support::service(Arc::clone(&counter), None);
    let mut request = support::body();
    let error = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap_err();
    match error {
        IoPreCallError::RateLimit(limit) => {
            assert_eq!(limit.limit, 0);
            assert_eq!(limit.current, 1);
        }
        IoPreCallError::OutputCap { field } => panic!("unexpected cap error for {field}"),
    }
    assert_eq!(counter.value(&support::keys(&deployment).itpm), 0);
}

async fn negative_limit_always_fails() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let deployment = support::deployment(Some(-5), None);
    let reservations = support::service(counter, Some(10));
    let mut request = support::body();
    let error = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap_err();
    match error {
        IoPreCallError::RateLimit(limit) => {
            assert_eq!(limit.limit, -5);
            assert_eq!(limit.current, 10);
        }
        IoPreCallError::OutputCap { field } => panic!("unexpected cap error for {field}"),
    }
}

async fn negative_max_tokens_reserves_nothing() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let deployment = support::deployment(None, Some(0));
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), support::number(-5));
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reserved.otpm_reserved, 0);
    assert_eq!(counter.value(&support::keys(&deployment).otpm), 0);
}

async fn float_max_tokens_is_an_error() {
    let value = Json::Number(Number::from_f64(20.5).unwrap());
    assert_eq!(cap_error(value).await, "max_tokens");
}

async fn text_max_tokens_is_an_error() {
    let value = Json::Text("20".to_owned());
    assert_eq!(cap_error(value).await, "max_tokens");
}

async fn bool_max_tokens_is_an_error() {
    let value = Json::Bool(true);
    assert_eq!(cap_error(value).await, "max_tokens");
}

async fn cap_error(value: Json) -> &'static str {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = support::deployment(None, Some(100));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), value);
    match reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap_err()
    {
        IoPreCallError::OutputCap { field } => field,
        IoPreCallError::RateLimit(limit) => panic!("unexpected rate limit {limit}"),
    }
}

async fn null_max_tokens_uses_next_field() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = support::deployment(None, Some(100));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), Json::Null);
    request.insert("max_completion_tokens".to_owned(), support::number(7));
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reserved.otpm_reserved, 7);
}

async fn model_cap_equal_to_limit_passes() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = support::deployment(None, Some(0x4000));
    let mut request = support::body();
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reserved.otpm_reserved, 0x4000);
}

async fn model_cap_over_limit_by_one_fails() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = support::deployment(None, Some(0x3FFF));
    let mut request = support::body();
    let error = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap_err();
    match error {
        IoPreCallError::RateLimit(limit) => {
            assert_eq!(limit.limit, 0x3FFF);
            assert_eq!(limit.current, 0x4000);
        }
        IoPreCallError::OutputCap { field } => panic!("unexpected cap error for {field}"),
    }
}

async fn missing_model_id_skips_limits() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let deployment = IoDeployment {
        model_id: None,
        model: Some("gpt-4o"),
        limits: support::deployment(Some(100), Some(100)).limits,
    };
    let mut request = support::body();
    assert!(
        reservations
            .pre_call_check(&deployment, &mut request)
            .await
            .unwrap()
            .is_none()
    );
    assert!(counter.calls().is_empty());
}

async fn missing_model_skips_limits() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let deployment = IoDeployment {
        model_id: Some("dep-a"),
        model: None,
        limits: support::deployment(Some(100), Some(100)).limits,
    };
    let mut request = support::body();
    assert!(
        reservations
            .pre_call_check(&deployment, &mut request)
            .await
            .unwrap()
            .is_none()
    );
    assert!(counter.calls().is_empty());
}

async fn no_limits_skips_everything() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let deployment = support::deployment(None, None);
    let mut request = support::body();
    assert!(
        reservations
            .pre_call_check(&deployment, &mut request)
            .await
            .unwrap()
            .is_none()
    );
    assert!(counter.calls().is_empty());
}

async fn empty_model_id_still_builds_keys() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = IoDeployment {
        model_id: Some(""),
        model: Some("gpt-4o"),
        limits: support::deployment(Some(100), None).limits,
    };
    let mut request = support::body();
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reserved.itpm_key,
        Some(format!("global_router::gpt-4o:itpm:{}", support::minute()))
    );
}

fn minute_labels() {
    let at = |seconds: u64| {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(seconds))
            .unwrap()
    };
    assert_eq!(minute_label(at(0)), "00-00");
    assert_eq!(minute_label(at(86_399)), "23-59");
    assert_eq!(minute_label(at(86_400)), "00-00");
    assert_eq!(minute_label(at(1_700_000_000)), "22-13");
    assert_eq!(
        minute_label(UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap()),
        "00-00"
    );
    assert_eq!(seconds_until_minute_reset(at(0)), 60);
    assert_eq!(seconds_until_minute_reset(at(30)), 30);
    assert_eq!(seconds_until_minute_reset(at(59)), 1);
    assert_eq!(seconds_until_minute_reset(at(86_399)), 1);
}

fn reservation_json(reserved: i64) -> Json {
    let mut metadata = IndexMap::new();
    metadata.insert(ITPM_RESERVED_KEY.to_owned(), support::number(reserved));
    metadata.insert(OTPM_RESERVED_KEY.to_owned(), support::number(0));
    metadata.insert(ITPM_CACHE_KEY.to_owned(), Json::Text("k".to_owned()));
    metadata.insert(OTPM_CACHE_KEY.to_owned(), Json::Null);
    Json::Object(metadata)
}

fn reservation_channel_priority() {
    let mut request = IndexMap::new();
    request.insert("metadata".to_owned(), reservation_json(1));
    request.insert("litellm_metadata".to_owned(), reservation_json(2));
    let mut params = IndexMap::new();
    params.insert("metadata".to_owned(), reservation_json(3));
    request.insert("litellm_params".to_owned(), Json::Object(params));
    let mut logging = IndexMap::new();
    logging.insert("metadata".to_owned(), reservation_json(4));
    request.insert("standard_logging_object".to_owned(), Json::Object(logging));
    assert_eq!(read_reservation(&request).unwrap().itpm_reserved, 1);
    request.shift_remove("metadata");
    assert_eq!(read_reservation(&request).unwrap().itpm_reserved, 2);
    request.shift_remove("litellm_metadata");
    assert_eq!(read_reservation(&request).unwrap().itpm_reserved, 3);
    request.shift_remove("litellm_params");
    assert_eq!(read_reservation(&request).unwrap().itpm_reserved, 4);
}

fn channel_without_itpm_sentinel_is_skipped() {
    let mut request = IndexMap::new();
    let mut only_otpm = IndexMap::new();
    only_otpm.insert(OTPM_RESERVED_KEY.to_owned(), support::number(9));
    request.insert("metadata".to_owned(), Json::Object(only_otpm));
    request.insert("litellm_metadata".to_owned(), reservation_json(2));
    assert_eq!(read_reservation(&request).unwrap().itpm_reserved, 2);
}

fn clear_removes_forged_sentinels() {
    let mut request = IndexMap::new();
    let mut forged = IndexMap::new();
    forged.insert(ITPM_RESERVED_KEY.to_owned(), support::number(999));
    forged.insert("keep".to_owned(), Json::Text("x".to_owned()));
    request.insert("metadata".to_owned(), Json::Object(forged));
    let mut params = IndexMap::new();
    let mut params_metadata = IndexMap::new();
    params_metadata.insert(OTPM_RESERVED_KEY.to_owned(), support::number(5));
    params.insert("metadata".to_owned(), Json::Object(params_metadata));
    request.insert("litellm_params".to_owned(), Json::Object(params));
    clear_reservation(&mut request);
    assert!(read_reservation(&request).is_none());
    let Some(Json::Object(cleaned)) = request.get("metadata") else {
        panic!("metadata is an object");
    };
    assert!(cleaned.contains_key("keep"));
    assert!(!cleaned.contains_key(ITPM_RESERVED_KEY));
}

fn stash_merges_existing_metadata() {
    let mut request = IndexMap::new();
    let mut existing = IndexMap::new();
    existing.insert("trace".to_owned(), Json::Text("keep".to_owned()));
    request.insert("litellm_metadata".to_owned(), Json::Object(existing));
    let reservation = IoReservation {
        itpm_reserved: 3,
        otpm_reserved: 4,
        itpm_key: Some("ik".to_owned()),
        otpm_key: None,
    };
    stash_reservation(&mut request, &reservation);
    assert_eq!(read_reservation(&request), Some(reservation));
    let Some(Json::Object(stashed)) = request.get("metadata") else {
        panic!("metadata is an object");
    };
    assert_eq!(stashed.get(ITPM_RESERVED_KEY), Some(&support::number(3)));
    assert_eq!(stashed.get(OTPM_CACHE_KEY), Some(&Json::Null));
    let Some(Json::Object(merged)) = request.get("litellm_metadata") else {
        panic!("litellm_metadata is an object");
    };
    assert_eq!(merged.get("trace"), Some(&Json::Text("keep".to_owned())));
    assert!(merged.contains_key(ITPM_RESERVED_KEY));
}

fn stash_replaces_non_object_metadata() {
    let mut request = IndexMap::new();
    request.insert("metadata".to_owned(), Json::Text("not-a-map".to_owned()));
    request.insert(
        "litellm_metadata".to_owned(),
        Json::Text("also-not-a-map".to_owned()),
    );
    let reservation = IoReservation {
        itpm_reserved: 1,
        otpm_reserved: 0,
        itpm_key: None,
        otpm_key: None,
    };
    stash_reservation(&mut request, &reservation);
    assert_eq!(read_reservation(&request), Some(reservation));
    assert_eq!(
        request.get("litellm_metadata"),
        Some(&Json::Text("also-not-a-map".to_owned()))
    );
}

fn billable_input_never_negative() {
    let usage = UsageTokens {
        prompt_tokens: 2,
        completion_tokens: 0,
        cached_tokens: 5,
    };
    assert_eq!(usage.billable_input(), 0);
}

fn standard_logging_zero_tokens_unresolved() {
    let usage = resolve_reconcile_usage(None, Some(UsageTokens::default()));
    assert!(!usage.usage_resolved);
}

async fn cancelled_output_increment_keeps_input() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Pending));
    let deployment = support::deployment(Some(100), Some(50));
    let keys = support::keys(&deployment);
    counter.seed(&keys.itpm, 5);
    counter.seed(&keys.otpm, 7);
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), support::number(20));
    let outcome = timeout(
        Duration::ZERO,
        reservations.pre_call_check(&deployment, &mut request),
    )
    .await;
    outcome.unwrap_err();
    assert_eq!(counter.value(&keys.itpm), 15);
    assert_eq!(counter.value(&keys.otpm), 7);
    assert!(read_reservation(&request).is_none());
}
