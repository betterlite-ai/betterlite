use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::batch_reads::{
    BatchReadOptions, BatchReads, BatchReadsError, DeclaredBatchRead, PendingBatchRead,
};
use betterlite_cache::dual_cache::{CacheOptions, DualCache, DualCacheSettings};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redis::batch::RedisBatch;
use rstest::rstest;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_group_usage::counters::BatchCounters;
use crate::model_group_usage::usage::{
    IoUsage, Usage, get_model_group_io_token_usage, get_model_group_usage,
};

use super::support::{
    CacheFailure, FakeCache, answered, deployment, float, itpm_key, now, number, otpm_key, rpm_key,
    runtime, text, tpm_key,
};

// a cache whose batch read does not depend on any tier, so the options it receives are the only thing to observe
struct RecordingCache {
    options: Mutex<Vec<BatchReadOptions>>,
}

impl BatchReads for RecordingCache {
    fn reserve_redis_batch_keys(
        &self,
        _current_time: f64,
        _keys: &[String],
        _result: &[Option<Json>],
    ) -> (Vec<String>, BTreeMap<String, Option<f64>>) {
        (Vec::new(), BTreeMap::new())
    }

    fn rollback_redis_batch_keys(&self, _previous_access_times: &BTreeMap<String, Option<f64>>) {}

    fn reserve_redis_batch_reads(
        &self,
        _keys: &[String],
    ) -> (Vec<String>, BTreeMap<String, Option<f64>>) {
        (Vec::new(), BTreeMap::new())
    }

    fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<Vec<Option<Json>>> {
        None
    }

    fn prepare_batch_get(
        &self,
        _keys: &[String],
        _options: BatchReadOptions,
    ) -> Result<PendingBatchRead, BatchReadsError> {
        Err(BatchReadsError::UnrepresentableValue)
    }

    fn apply_batch_get(
        &self,
        _pending: &PendingBatchRead,
        _redis_result: Option<&BTreeMap<String, Option<Json>>>,
    ) -> Result<Vec<Option<Json>>, BatchReadsError> {
        Ok(Vec::new())
    }

    fn async_batch_get_cache(
        &self,
        keys: &[String],
        options: BatchReadOptions,
    ) -> impl Future<Output = Option<Vec<Option<Json>>>> + Send {
        self.options
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(options);
        let absent: Vec<Option<Json>> = keys.iter().map(|_key| None).collect();
        ready(Some(absent))
    }

    fn declare_batch_get(
        &self,
        _keys: &[String],
        _batch: &Arc<RedisBatch>,
    ) -> Result<DeclaredBatchRead, BatchReadsError> {
        Err(BatchReadsError::UnrepresentableValue)
    }

    fn async_resolve_batch_get(
        _declared: DeclaredBatchRead,
    ) -> impl Future<Output = Result<Vec<Option<Json>>, BatchReadsError>> + Send {
        ready(Ok(Vec::new()))
    }

    fn async_batch_get_cache_shared(
        _reads: &[(&Self, &[String])],
    ) -> impl Future<Output = Vec<Option<Vec<Option<Json>>>>> + Send {
        ready(Vec::new())
    }
}

#[test]
fn batch_read_options_match_the_router_defaults() {
    let cache = RecordingCache {
        options: Mutex::new(Vec::new()),
    };
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(usage, Usage::default());
    let recorded = cache.options.lock().unwrap_or_else(PoisonError::into_inner);
    let options = recorded.first().expect("the batch read");
    assert!(!options.local_only);
    assert!(options.throttle_redis);
}

#[test]
fn batch_key_order() {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "b", "openai/gpt-4o"),
    ];
    let cache = FakeCache::answering(answered(vec![
        Json::Null,
        Json::Null,
        Json::Null,
        Json::Null,
    ]));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(usage, Usage::default());
    assert_eq!(
        cache.batches(),
        [vec![
            tpm_key("a", "openai/gpt-4o"),
            tpm_key("b", "openai/gpt-4o"),
            rpm_key("a", "openai/gpt-4o"),
            rpm_key("b", "openai/gpt-4o"),
        ]]
    );
}

#[test]
fn io_batch_key_order_sends_every_itpm_key_before_every_otpm_key() {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "b", "openai/gpt-4o"),
    ];
    let cache = FakeCache::answering(answered(vec![
        Json::Null,
        Json::Null,
        Json::Null,
        Json::Null,
    ]));
    let usage = runtime()
        .block_on(get_model_group_io_token_usage(Some(&rows), &cache, now()))
        .expect("the io usage read");
    assert_eq!(usage, IoUsage::default());
    assert_eq!(
        cache.batches(),
        [vec![
            itpm_key("a", "openai/gpt-4o"),
            itpm_key("b", "openai/gpt-4o"),
            otpm_key("a", "openai/gpt-4o"),
            otpm_key("b", "openai/gpt-4o"),
        ]]
    );
}

#[test]
fn skip_missing_identity() {
    let mut missing_id = deployment("group", "gone", "openai/gpt-4o");
    missing_id.model_info.shift_remove("id");
    let mut missing_model = deployment("group", "b", "openai/gpt-4o");
    missing_model.litellm_params.shift_remove("model");
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        missing_id,
        deployment("group", "", "openai/gpt-4o"),
        missing_model,
    ];
    let cache = FakeCache::answering(answered(vec![
        Json::Null,
        Json::Null,
        Json::Null,
        Json::Null,
    ]));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(usage, Usage::default());
    assert_eq!(
        cache.batches(),
        [vec![
            tpm_key("a", "openai/gpt-4o"),
            tpm_key("", "openai/gpt-4o"),
            rpm_key("a", "openai/gpt-4o"),
            rpm_key("", "openai/gpt-4o"),
        ]]
    );
}

#[test]
fn empty_batch() {
    let rows: [DeploymentSpec; 0] = [];
    let cache = FakeCache::answering(Some(Vec::new()));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(usage, Usage::default());
    assert_eq!(cache.batches(), [Vec::new()]);
}

#[test]
fn null_model_list_skips_the_cache() {
    let cache = FakeCache::failing();
    let usage = runtime()
        .block_on(get_model_group_usage(None, &cache, now()))
        .expect("the usage read");
    assert_eq!(usage, Usage::default());
    assert!(cache.batches().is_empty());
}

#[rstest]
#[case::shorter_than_the_first_side(vec![Some(number(5))], (Some(5), None))]
#[case::split_across_a_short_answer(
    vec![Some(number(5)), Some(number(7)), Some(number(11))],
    (Some(12), Some(11))
)]
#[case::the_whole_answer(
    vec![
        Some(number(5)),
        Some(number(7)),
        Some(number(11)),
        Some(number(13)),
    ],
    (Some(12), Some(24))
)]
#[case::no_values(vec![], (None, None))]
fn short_batch(#[case] values: Vec<Option<Json>>, #[case] expected: (Option<i64>, Option<i64>)) {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "b", "openai/gpt-4o"),
    ];
    let cache = FakeCache::answering(Some(values));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!((usage.tpm, usage.rpm), expected);
}

#[test]
fn boolean_and_invalid_counters_sum_only_python_ints() {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "b", "openai/gpt-4o"),
        deployment("group", "c", "openai/gpt-4o"),
        deployment("group", "d", "openai/gpt-4o"),
    ];
    let cache = FakeCache::answering(answered(vec![
        Json::Bool(true),
        float(1.5),
        number(2),
        text("3"),
        Json::Bool(false),
        Json::List(Vec::new()),
        Json::Null,
        Json::Object(JsonObject::new()),
    ]));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(
        usage,
        Usage {
            tpm: Some(3),
            rpm: Some(0)
        }
    );
}

#[rstest]
#[case::a_zero_counter_is_usage(vec![Some(number(0)), Some(Json::Null)], (Some(0), None))]
#[case::no_counters_is_none(vec![Some(Json::Null), Some(Json::Null)], (None, None))]
#[case::the_rpm_side_can_be_zero_too(vec![Some(Json::Null), Some(number(0))], (None, Some(0)))]
fn zero_versus_none(
    #[case] values: Vec<Option<Json>>,
    #[case] expected: (Option<i64>, Option<i64>),
) {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answering(Some(values));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!((usage.tpm, usage.rpm), expected);
}

#[test]
fn duplicate_deployment_sum_counts_every_key_in_the_batch() {
    let rows = [
        deployment("group", "a", "openai/gpt-4o"),
        deployment("group", "a", "openai/gpt-4o"),
    ];
    let cache = FakeCache::answering(answered(vec![number(1), number(2), number(3), number(4)]));
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &cache, now()))
        .expect("the usage read");
    assert_eq!(
        usage,
        Usage {
            tpm: Some(3),
            rpm: Some(7)
        }
    );
    let batches = cache.batches();
    let first = batches.first().expect("the batch read");
    assert_eq!(
        first,
        &[
            tpm_key("a", "openai/gpt-4o"),
            tpm_key("a", "openai/gpt-4o"),
            rpm_key("a", "openai/gpt-4o"),
            rpm_key("a", "openai/gpt-4o"),
        ]
    );
}

#[test]
fn batch_error() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::failing();
    let standard = runtime().block_on(get_model_group_usage(Some(&rows), &cache, now()));
    assert!(matches!(standard, Err(CacheFailure)));
    let io = runtime().block_on(get_model_group_io_token_usage(Some(&rows), &cache, now()));
    assert!(matches!(io, Err(CacheFailure)));
}

#[test]
fn reads_counters_from_the_dual_cache() {
    let memory =
        LocalMemoryOps::new(1024, Arc::new(|| 0.0_f64)).expect("the local memory tier builds");
    let cache: DualCache<LocalMemoryOps> = DualCache::new(
        Arc::new(memory),
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: 1000,
            redis_delete_chunk_size: 1000,
        },
    );
    cache.set_cache(
        &tpm_key("a", "openai/gpt-4o"),
        &number(7),
        CacheOptions::default(),
    );
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let counters = BatchCounters { cache: &cache };
    let usage = runtime()
        .block_on(get_model_group_usage(Some(&rows), &counters, now()))
        .expect("the usage read");
    assert_eq!(
        usage,
        Usage {
            tpm: Some(7),
            rpm: None
        }
    );
}
