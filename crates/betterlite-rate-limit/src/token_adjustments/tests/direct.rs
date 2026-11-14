#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use super::support::{
    FakeBatch, FakeCache, FakeScript, FakeUsage, adjustments_with, operation, pipeline_tuples,
    script_handle, without_scripts,
};
use crate::rate_limit_window_store::RedisCacheKind;
use crate::token_adjustments::direct::{
    execute_token_increment_script, increment_tokens_keeping_ttl,
};
use crate::token_adjustments::scripts::TOKEN_INCREMENT_SCRIPT;

#[tokio::test]
async fn an_empty_operation_list_runs_nothing() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        None,
    );
    increment_tokens_keeping_ttl(&adjustments, &[]).await;
    assert!(script.calls().is_empty());
    assert!(usage.pipelines().is_empty());
}

#[tokio::test]
async fn without_the_script_every_operation_takes_the_regular_pipeline() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let adjustments = without_scripts(&cache, &usage);
    let operations = [
        operation("{a:1}:tokens", 4.0, Some(60)),
        operation("{a:1}:requests", 1.0, None),
    ];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    assert_eq!(
        pipeline_tuples(&usage.pipelines()),
        vec![vec![
            ("{a:1}:tokens".to_owned(), 4.0, Some(60)),
            ("{a:1}:requests".to_owned(), 1.0, None),
        ]]
    );
}

#[tokio::test]
async fn a_standalone_cache_runs_one_script_for_every_operation() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        None,
    );
    let operations = [
        operation("{a:1}:tokens", 4.0, Some(60)),
        operation("{a:1}:requests", 1.0, None),
    ];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    assert_eq!(
        script.calls(),
        vec![(
            vec!["{a:1}:tokens".to_owned(), "{a:1}:requests".to_owned()],
            vec![
                RedisScalar::Float(4.0),
                RedisScalar::Text("60".to_owned()),
                RedisScalar::Float(1.0),
                RedisScalar::Text("0".to_owned()),
            ],
        )]
    );
    assert!(usage.pipelines().is_empty());
}

#[tokio::test]
async fn a_cluster_cache_runs_one_script_per_hash_tag_group() {
    let cache = FakeCache::new(Some(RedisCacheKind::Cluster));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        None,
    );
    let operations = [
        operation("{foo}:tokens", 4.0, Some(60)),
        operation("{bar}:tokens", 2.0, Some(60)),
    ];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    assert_eq!(
        script.calls(),
        vec![
            (
                vec!["{foo}:tokens".to_owned()],
                vec![RedisScalar::Float(4.0), RedisScalar::Text("60".to_owned()),],
            ),
            (
                vec!["{bar}:tokens".to_owned()],
                vec![RedisScalar::Float(2.0), RedisScalar::Text("60".to_owned()),],
            ),
        ]
    );
    assert!(usage.pipelines().is_empty());
}

#[tokio::test]
async fn a_failed_script_run_falls_back_to_the_regular_pipeline() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    script.fail();
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        None,
    );
    let operations = [operation("{a:1}:tokens", 4.0, Some(60))];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    assert_eq!(script.calls().len(), 1);
    assert_eq!(
        pipeline_tuples(&usage.pipelines()),
        vec![vec![("{a:1}:tokens".to_owned(), 4.0, Some(60))]]
    );
}

#[tokio::test]
async fn a_second_group_failing_falls_back_with_every_operation() {
    let cache = FakeCache::new(Some(RedisCacheKind::Cluster));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    script.fail_from(2);
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        None,
    );
    let operations = [
        operation("{foo}:tokens", 4.0, Some(60)),
        operation("{bar}:tokens", 2.0, Some(60)),
    ];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    assert_eq!(script.calls().len(), 2);
    assert_eq!(
        pipeline_tuples(&usage.pipelines()),
        vec![vec![
            ("{foo}:tokens".to_owned(), 4.0, Some(60)),
            ("{bar}:tokens".to_owned(), 2.0, Some(60)),
        ]]
    );
}

#[tokio::test]
async fn an_open_post_call_batch_takes_each_group_instead_of_running_it() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    let batch = FakeBatch::new();
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        Some(batch.clone()),
    );
    let operations = [operation("{a:1}:tokens", 4.0, Some(60))];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    assert!(script.calls().is_empty());
    assert!(usage.pipelines().is_empty());
    assert_eq!(
        batch.calls(),
        vec![(
            TOKEN_INCREMENT_SCRIPT.to_owned(),
            vec!["{a:1}:tokens".to_owned()],
            vec![RedisScalar::Float(4.0), RedisScalar::Text("60".to_owned()),],
        )]
    );
}

#[tokio::test]
async fn executing_without_a_registered_script_does_nothing() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let adjustments = without_scripts(&cache, &usage);
    let operations = [operation("{a:1}:tokens", 4.0, Some(60))];
    execute_token_increment_script(&adjustments, &operations)
        .await
        .expect("the absent script path");
    assert!(usage.pipelines().is_empty());
    assert!(usage.increments().is_empty());
}
