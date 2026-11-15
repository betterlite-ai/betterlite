#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::Settled;
use betterlite_redis::cache_primitives::protocol::RedisScalar;

use super::support::{
    FakeBatch, FakeCache, FakeScript, FakeUsage, adjustments_with, batch_handle, operation,
    pipeline_tuples, script_handle,
};
use crate::limiter_records::ScriptReply;
use crate::rate_limit_window_store::RedisCacheKind;
use crate::token_adjustments::deferred::defer_token_increment_script;
use crate::token_adjustments::direct::increment_tokens_keeping_ttl;
use crate::token_adjustments::scripts::TOKEN_INCREMENT_SCRIPT;

fn failed_script() -> Settled<ScriptReply> {
    Err(Arc::new(BatchError::MissingReply {
        operation: "token increment",
    }))
}

fn cancelled_script() -> Settled<ScriptReply> {
    Err(Arc::new(BatchError::Cancelled))
}

#[tokio::test]
async fn a_deferral_without_the_script_is_not_taken() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let batch = FakeBatch::new();
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        None,
        None,
        Some(batch_handle(&batch)),
    );
    let keys = vec!["{alpha}:tokens".to_owned()];
    let args = vec![RedisScalar::Float(4.0), RedisScalar::Text("60".to_owned())];
    let operations = [operation("{alpha}:tokens", 4.0, Some(60))];
    assert!(!defer_token_increment_script(
        &adjustments,
        &keys,
        &args,
        &operations
    ));
    assert!(batch.calls().is_empty());
}

#[tokio::test]
async fn a_deferral_without_an_open_batch_is_not_taken() {
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
    let keys = vec!["{alpha}:tokens".to_owned()];
    let args = vec![RedisScalar::Float(4.0), RedisScalar::Text("60".to_owned())];
    let operations = [operation("{alpha}:tokens", 4.0, Some(60))];
    assert!(!defer_token_increment_script(
        &adjustments,
        &keys,
        &args,
        &operations
    ));
}

#[tokio::test]
async fn a_deferral_with_the_script_and_an_open_batch_is_taken() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    let batch = FakeBatch::new();
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        Some(batch_handle(&batch)),
    );
    let keys = vec!["{alpha}:tokens".to_owned()];
    let args = vec![RedisScalar::Float(4.0), RedisScalar::Text("60".to_owned())];
    let operations = [operation("{alpha}:tokens", 4.0, Some(60))];
    assert!(defer_token_increment_script(
        &adjustments,
        &keys,
        &args,
        &operations
    ));
    assert_eq!(batch.calls().len(), 1);
}

#[tokio::test]
async fn a_settled_deferral_that_succeeded_leaves_the_pipeline_alone() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    let batch = FakeBatch::new();
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        Some(batch_handle(&batch)),
    );
    let operations = [operation("{alpha}:tokens", 4.0, Some(60))];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    batch.settle(&Ok(ScriptReply::Array(Vec::new()))).await;
    assert!(usage.pipelines().is_empty());
}

#[tokio::test]
async fn a_settled_deferral_that_failed_runs_the_regular_pipeline() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    let batch = FakeBatch::new();
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        Some(batch_handle(&batch)),
    );
    let operations = [operation("{alpha}:tokens", 4.0, Some(60))];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    batch.settle(&failed_script()).await;
    assert_eq!(
        pipeline_tuples(&usage.pipelines()),
        vec![vec![("{alpha}:tokens".to_owned(), 4.0_f64, Some(60))]]
    );
}

#[tokio::test]
async fn a_cancelled_deferral_does_not_run_the_regular_pipeline() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    let batch = FakeBatch::new();
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        Some(batch_handle(&batch)),
    );
    let operations = [operation("{alpha}:tokens", 4.0, Some(60))];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    batch.settle(&cancelled_script()).await;
    assert!(usage.pipelines().is_empty());
}

#[tokio::test]
async fn each_deferred_group_carries_its_own_fallback_operations() {
    let cache = FakeCache::new(Some(RedisCacheKind::Cluster));
    let usage = FakeUsage::new();
    let script = FakeScript::new(TOKEN_INCREMENT_SCRIPT);
    let batch = FakeBatch::new();
    let adjustments = adjustments_with(
        Arc::clone(&cache),
        Arc::clone(&usage),
        Some(script_handle(&script)),
        None,
        Some(batch_handle(&batch)),
    );
    let operations = [
        operation("{foo}:tokens", 4.0, Some(60)),
        operation("{bar}:tokens", 2.0, Some(60)),
    ];
    increment_tokens_keeping_ttl(&adjustments, &operations).await;
    assert_eq!(batch.calls().len(), 2);
    batch.settle(&failed_script()).await;
    assert_eq!(
        pipeline_tuples(&usage.pipelines()),
        vec![
            vec![("{foo}:tokens".to_owned(), 4.0_f64, Some(60))],
            vec![("{bar}:tokens".to_owned(), 2.0_f64, Some(60))],
        ]
    );
}
