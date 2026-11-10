#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use redis::Value;
use rstest::rstest;

use self::support::{Harness, cluster_harness, harness, unreachable_harness};
use super::RedisBatch;
use super::error::{BatchError, PipelineFailure};
use super::result::{BatchResult, SettledHook, SettledStatus};
use crate::cache_primitives::decode::CachedValue;
use crate::redis_counters::scripts::RegisteredScript;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

type MgetValues = BTreeMap<String, Option<CachedValue>>;
type MgetHandle = BatchResult<MgetValues>;
type DeclaredHandles = Arc<Mutex<Vec<MgetHandle>>>;

#[derive(Clone, Copy, Debug)]
enum Behavior {
    SuccessfulOperations,
    PipelineFailure,
    StandaloneRecovery,
    MissTracking,
}

#[rstest]
#[case::successful_operations(Behavior::SuccessfulOperations)]
#[case::pipeline_failure(Behavior::PipelineFailure)]
#[case::standalone_recovery(Behavior::StandaloneRecovery)]
#[case::miss_tracking(Behavior::MissTracking)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn batch_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::SuccessfulOperations => successful_operations().await,
        Behavior::PipelineFailure => pipeline_failure().await,
        Behavior::StandaloneRecovery => standalone_recovery().await,
        Behavior::MissTracking => miss_tracking().await,
    }
}

#[derive(Clone, Copy, Debug)]
enum Boundary {
    MgetReplyShape,
    IncrementReplyShape,
    MissingKeys,
    Cancellation,
    ClusterStandalone,
    SettledHookOrder,
    FlushHookOrder,
}

#[rstest]
#[case::mget_reply_shape(Boundary::MgetReplyShape)]
#[case::increment_reply_shape(Boundary::IncrementReplyShape)]
#[case::missing_keys(Boundary::MissingKeys)]
#[case::cancellation(Boundary::Cancellation)]
#[case::cluster_standalone(Boundary::ClusterStandalone)]
#[case::settled_hook_order(Boundary::SettledHookOrder)]
#[case::flush_hook_order(Boundary::FlushHookOrder)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn batch_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::MgetReplyShape => mget_reply_shape().await,
        Boundary::IncrementReplyShape => increment_reply_shape().await,
        Boundary::MissingKeys => missing_keys().await,
        Boundary::Cancellation => cancellation().await,
        Boundary::ClusterStandalone => cluster_standalone().await,
        Boundary::SettledHookOrder => settled_hook_order().await,
        Boundary::FlushHookOrder => flush_hook_order().await,
    }
}

struct Declared {
    mget: MgetHandle,
    increment: BatchResult<f64>,
    set: BatchResult<()>,
    delete: BatchResult<()>,
    script: BatchResult<Value>,
}

fn batch(harness: &Harness) -> Arc<RedisBatch> {
    Arc::new(RedisBatch::new(Arc::clone(&harness.cache), "redis_batch"))
}

fn registered(harness: &Harness, source: &str) -> Arc<RegisteredScript> {
    Arc::new(harness.cache.counters.async_register_script(source))
}

fn cached(value: &str) -> CachedValue {
    CachedValue::Text(value.to_owned())
}

fn commands(server: &FakeServer, names: &[&str]) -> Vec<String> {
    server
        .commands()
        .into_iter()
        .filter(|command| names.contains(&command.name.as_str()))
        .map(|command| command.name)
        .collect()
}

fn log_entries(log: &Mutex<Vec<String>>) -> Vec<String> {
    match log.lock() {
        Ok(entries) => entries.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn recording_hook(name: &'static str, log: Arc<Mutex<Vec<String>>>) -> SettledHook {
    Arc::new(move |status: SettledStatus| {
        let text = match status {
            SettledStatus::Completed => "completed",
            SettledStatus::Failed => "failed",
            SettledStatus::Cancelled => "cancelled",
        };
        match log.lock() {
            Ok(mut entries) => entries.push(format!("{name}:{text}")),
            Err(poisoned) => poisoned.into_inner().push(format!("{name}:{text}")),
        }
        None
    })
}

fn script_successful_replies(server: &FakeServer) {
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("1".to_owned()), Reply::Nil]),
    );
    server.script("INCRBYFLOAT", Reply::Bulk("2.5".to_owned()));
    server.script("EXPIRE", Reply::Int(1));
    server.script("SET", Reply::Simple("OK".to_owned()));
    server.script("DEL", Reply::Int(1));
    server.script("EVALSHA", Reply::Int(7));
}

fn declare_successful_operations(
    batch: &Arc<RedisBatch>,
    script: &Arc<RegisteredScript>,
) -> Declared {
    Declared {
        mget: batch.mget(&["a".to_owned(), "b".to_owned()]),
        increment: batch.increment("c", 2.5, Some(20)),
        set: batch.set("d", "value", Some(5.0)),
        delete: batch.delete("e"),
        script: batch.script(script, &["f".to_owned()], &[]),
    }
}

async fn assert_successful_results(declared: Declared) {
    let values = declared.mget.await.expect("the mget");
    assert_eq!(values.get("a"), Some(&Some(cached("1"))));
    assert_eq!(values.get("b"), Some(&None));
    assert_eq!(
        declared.increment.await.expect("the increment").to_bits(),
        2.5_f64.to_bits()
    );
    declared.set.await.expect("the set");
    declared.delete.await.expect("the delete");
    assert_eq!(declared.script.await.expect("the script"), Value::Int(7));
}

fn assert_successful_commands(server: &FakeServer, script: &RegisteredScript) {
    let mgets = server.commands_named("MGET");
    assert_eq!(mgets.len(), 1);
    assert_eq!(mgets[0].args, ["MGET", "ns:a", "ns:b"]);
    assert_eq!(
        server.commands_named("INCRBYFLOAT")[0].args,
        ["INCRBYFLOAT", "ns:c", "2.5"]
    );
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "ns:c", "20"]
    );
    assert_eq!(
        server.commands_named("SET")[0].args,
        ["SET", "ns:d", "\"value\"", "EX", "5"]
    );
    assert_eq!(server.commands_named("DEL")[0].args, ["DEL", "ns:e"]);
    let evalshas = server.commands_named("EVALSHA");
    assert_eq!(
        evalshas[0].args,
        ["EVALSHA", script.sha1.as_str(), "1", "ns:f"]
    );
}

async fn successful_operations() {
    let server = FakeServer::start().await;
    script_successful_replies(&server);
    let harness = harness(&server, Some("ns"));
    let batch = batch(&harness);
    let script = registered(&harness, "return 7");
    let declared = declare_successful_operations(&batch, &script);
    assert_eq!(batch.pending(), 5);

    assert_successful_results(declared).await;
    assert_eq!(batch.flushes(), 1);
    assert_eq!(batch.pending(), 0);
    assert_successful_commands(&server, &script);
    assert_eq!(
        harness.hooks.for_call("redis_batch"),
        ["success:redis_batch"]
    );
}

async fn pipeline_failure() {
    let harness = unreachable_harness();
    let batch = batch(&harness);
    let mget = batch.mget(&["a".to_owned()]);
    let increment = batch.increment("b", 1.5, None);

    let first = mget.await.expect_err("the failed mget");
    let second = increment.await.expect_err("the failed increment");
    assert!(matches!(
        first.as_ref(),
        BatchError::Pipeline(PipelineFailure::Client(_))
    ));
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(batch.flushes(), 1);
    let hooks = harness.hooks.for_call("redis_batch");
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:redis_batch"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:redis_batch");
    assert_eq!(gate.len(), 2);
    assert!(gate[1].starts_with("failure:"));
}

async fn standalone_recovery() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"));
    let script = registered(&harness, "return 7");
    server.script("MGET", Reply::Array(vec![Reply::Bulk("9".to_owned())]));
    let noscript = Reply::Error("NOSCRIPT No matching script. Please use EVAL.".to_owned());
    server.script("EVALSHA", noscript);
    server.script(
        "EVALSHA",
        Reply::Error("NOSCRIPT No matching script. Please use EVAL.".to_owned()),
    );
    server.script("SCRIPT", Reply::Bulk(script.sha1.clone()));
    server.script("EVALSHA", Reply::Int(7));
    let batch = batch(&harness);

    let mget = batch.mget(&["key".to_owned()]);
    let scripted = batch.script(&script, &["counter".to_owned()], &[]);
    let values = mget.await.expect("the mget");
    assert_eq!(values.get("key"), Some(&Some(cached("9"))));
    assert_eq!(scripted.await.expect("the recovered script"), Value::Int(7));
    assert_eq!(
        commands(&server, &["MGET", "EVALSHA", "SCRIPT"]),
        ["MGET", "EVALSHA", "EVALSHA", "SCRIPT", "EVALSHA"]
    );
    assert_eq!(batch.flushes(), 1);
}

async fn miss_tracking() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Nil, Reply::Bulk("2".to_owned())]),
    );
    server.script("MGET", Reply::Array(vec![Reply::Bulk("9".to_owned())]));
    server.script("SET", Reply::Simple("OK".to_owned()));
    server.script("DEL", Reply::Int(1));
    let harness = harness(&server, Some("ns"));
    let batch = batch(&harness);

    let first = batch.mget(&["a".to_owned(), "b".to_owned()]);
    let values = first.await.expect("the mget");
    assert_eq!(values.get("a"), Some(&None));
    assert_eq!(values.get("b"), Some(&Some(cached("2"))));
    assert!(batch.read_as_missing("a"));
    assert!(!batch.read_as_missing("b"));

    let set = batch.set("a", "written", None);
    assert!(!batch.read_as_missing("a"));
    let delete = batch.delete("b");
    assert!(batch.read_as_missing("b"));
    set.await.expect("the set");
    delete.await.expect("the delete");

    let second = batch.mget(&["a".to_owned()]);
    let rediscovered = second.await.expect("the rediscovered mget");
    assert_eq!(rediscovered.get("a"), Some(&Some(cached("9"))));
    assert_eq!(batch.flushes(), 3);
}

async fn mget_reply_shape() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Simple("OK".to_owned()));
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let harness = harness(&server, Some("ns"));
    let batch = batch(&harness);

    let mget = batch.mget(&["a".to_owned()]);
    let increment = batch.increment("b", 1.5, None);
    let error = mget.await.expect_err("the shaped mget");
    let BatchError::MgetReply { kind } = error.as_ref() else {
        panic!("expected an MGET shape error, got {error:?}");
    };
    assert_eq!(*kind, "a simple string");
    assert_eq!(
        increment.await.expect("the increment").to_bits(),
        1.5_f64.to_bits()
    );
}

async fn increment_reply_shape() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Nil);
    let harness = harness(&server, Some("ns"));
    let batch = batch(&harness);

    let increment = batch.increment("b", 1.5, None);
    let error = increment.await.expect_err("the shaped increment");
    let BatchError::IncrementReply { kind } = error.as_ref() else {
        panic!("expected an increment shape error, got {error:?}");
    };
    assert_eq!(*kind, "nil");
}

async fn missing_keys() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Error("ERR broken".to_owned()));
    let harness = cluster_harness(&server, Some("ns"));
    let batch = batch(&harness);

    let mget = batch.mget(&["alpha".to_owned(), "beta".to_owned()]);
    let error = mget.await.expect_err("the missing keys");
    let BatchError::MissingKeys {
        requested,
        returned,
    } = error.as_ref()
    else {
        panic!("expected a missing keys error, got {error:?}");
    };
    assert_eq!(*requested, 2);
    assert_eq!(*returned, 0);
}

async fn cancellation() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Array(vec![Reply::Nil]));
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let harness = harness(&server, Some("ns"));
    let batch = batch(&harness);
    let log = Arc::new(Mutex::new(Vec::new()));

    let mget = batch.mget(&["a".to_owned()]);
    mget.on_settled(recording_hook("mget", Arc::clone(&log)));
    let increment = batch.increment("b", 1.5, None);
    mget.cancel();
    assert!(mget.done());

    assert_eq!(
        increment.await.expect("the increment").to_bits(),
        1.5_f64.to_bits()
    );
    let cancelled = mget.await.expect_err("the cancelled mget");
    assert!(matches!(cancelled.as_ref(), BatchError::Cancelled));
    assert_eq!(batch.flushes(), 1);
    assert_eq!(log_entries(&log), ["mget:cancelled"]);
}

async fn cluster_standalone() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Array(vec![Reply::Bulk("1".to_owned())]));
    server.script("INCRBYFLOAT", Reply::Bulk("2.5".to_owned()));
    server.script("TTL", Reply::Int(-1));
    server.script("EXPIRE", Reply::Int(1));
    let harness = cluster_harness(&server, Some("ns"));
    let batch = batch(&harness);

    let mget = batch.mget(&["alpha".to_owned()]);
    let increment = batch.increment("beta", 2.5, None);
    let values = mget.await.expect("the cluster mget");
    assert_eq!(values.get("alpha"), Some(&Some(cached("1"))));
    assert_eq!(
        increment.await.expect("the cluster increment").to_bits(),
        2.5_f64.to_bits()
    );
    assert_eq!(batch.flushes(), 1);
    assert_eq!(batch.pending(), 0);

    // the TTL and EXPIRE commands only exist on the standalone increment path
    assert_eq!(server.commands_named("MGET").len(), 1);
    assert_eq!(server.commands_named("INCRBYFLOAT").len(), 1);
    assert_eq!(server.commands_named("TTL").len(), 1);
    assert_eq!(
        server.commands_named("EXPIRE")[0].args,
        ["EXPIRE", "ns:beta", "60"]
    );
}

async fn settled_hook_order() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Array(vec![Reply::Bulk("1".to_owned())]));
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let harness = harness(&server, Some("ns"));
    let batch = batch(&harness);
    let log = Arc::new(Mutex::new(Vec::new()));

    let mget = batch.mget(&["a".to_owned()]);
    mget.on_settled(recording_hook("mget", Arc::clone(&log)));
    let increment = batch.increment("b", 1.5, None);
    increment.on_settled(recording_hook("increment", Arc::clone(&log)));

    assert_eq!(
        increment.await.expect("the increment").to_bits(),
        1.5_f64.to_bits()
    );
    mget.await.expect("the mget");
    assert_eq!(log_entries(&log), ["mget:completed", "increment:completed"]);
}

fn install_declaring_hook(batch: &Arc<RedisBatch>, declared: &DeclaredHandles) {
    let shared = Arc::clone(declared);
    let weak = Arc::downgrade(batch);
    batch.add_flush_hook(Arc::new(move || {
        let Some(this) = Weak::upgrade(&weak) else {
            return;
        };
        let result = this.mget(&["hook".to_owned()]);
        match shared.lock() {
            Ok(mut results) => results.push(result),
            Err(poisoned) => poisoned.into_inner().push(result),
        }
    }));
}

async fn flush_hook_order() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("1".to_owned()), Reply::Nil]),
    );
    let harness = harness(&server, Some("ns"));
    let batch = batch(&harness);
    let declared: DeclaredHandles = Arc::new(Mutex::new(Vec::new()));
    install_declaring_hook(&batch, &declared);

    let direct = batch.mget(&["direct".to_owned()]);
    assert_eq!(batch.pending(), 1);
    let values = direct.await.expect("the direct mget");
    assert_eq!(values.get("direct"), Some(&Some(cached("1"))));
    assert_eq!(batch.pending(), 0);
    assert_eq!(batch.flushes(), 1);
    let mgets = server.commands_named("MGET");
    assert_eq!(mgets.len(), 1);
    assert_eq!(mgets[0].args, ["MGET", "ns:direct", "ns:hook"]);

    let hook_result = match declared.lock() {
        Ok(mut results) => results.pop(),
        Err(poisoned) => poisoned.into_inner().pop(),
    };
    let hook_values = hook_result
        .expect("the flush hook declared an operation")
        .await
        .expect("the hook mget");
    assert_eq!(hook_values.get("hook"), Some(&None));
    assert_eq!(batch.flushes(), 1);
}
