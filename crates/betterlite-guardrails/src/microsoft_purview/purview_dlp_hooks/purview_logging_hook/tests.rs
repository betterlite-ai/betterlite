#![cfg(test)]

use std::mem;
use std::sync::{Arc, Mutex};

use betterlite_core::routes::CallType;
use tokio::runtime::Builder;
use tokio::sync::oneshot;

use crate::microsoft_purview::purview_dlp_hooks::purview_async_logging_hook::{
    AuditEngine, DeferredCall,
};
use crate::microsoft_purview::purview_dlp_hooks::purview_logging_hook::{
    AuditSpawner, ThreadAuditSpawner, logging_hook,
};
use crate::microsoft_purview::purview_dlp_hooks::test_support::{
    Scan, Script, TestSink, cache, data, guardrail_at, passing_actions, record_base, start, value,
};

type Job = Box<dyn FnOnce() + Send + 'static>;

const CHAT_RESULT: &str = r#"{"id":"c1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hello"}}]}"#;

#[derive(Default)]
struct RecordingSpawner {
    jobs: Mutex<Vec<Job>>,
}

impl RecordingSpawner {
    fn take(&self) -> Vec<Job> {
        mem::take(&mut *self.jobs.lock().unwrap())
    }

    fn scheduled(&self) -> usize {
        self.jobs.lock().unwrap().len()
    }
}

impl AuditSpawner for RecordingSpawner {
    fn spawn(&self, job: Job) {
        self.jobs.lock().unwrap().push(job);
    }
}

fn engine(base: &str) -> AuditEngine {
    AuditEngine {
        guardrail: Arc::new(guardrail_at(base)),
        record_base: record_base(),
        cache: Arc::new(cache()),
        sink: Arc::new(TestSink::default()),
    }
}

fn call(kwargs: &str) -> DeferredCall {
    DeferredCall {
        kwargs: data(kwargs),
        result: value(CHAT_RESULT),
        call_type: Some(CallType::Completion),
    }
}

const AUDITED_KWARGS: &str = r#"{"model":"gpt","messages":[{"role":"user","content":"hello"}],"litellm_params":{"metadata":{"user_api_key_user_id":"user-1"}}}"#;

#[tokio::test]
async fn defers_to_the_async_hook_when_an_event_loop_is_running() {
    let mock = start(Script::passing());
    let engine = engine(&mock.base);
    let spawner = RecordingSpawner::default();
    let returned = logging_hook(&engine, call(AUDITED_KWARGS), &spawner);
    assert_eq!(spawner.scheduled(), 0);
    assert_eq!(mock.process_calls(), 0);
    assert_eq!(
        returned.kwargs.get("messages"),
        data(AUDITED_KWARGS).get("messages")
    );
}

#[test]
fn schedules_a_background_audit_without_an_event_loop() {
    let mock = start(Script::new(vec![passing_actions(), passing_actions()]));
    let engine = engine(&mock.base);
    let spawner = RecordingSpawner::default();
    let returned = logging_hook(&engine, call(AUDITED_KWARGS), &spawner);
    assert_eq!(spawner.scheduled(), 1);
    assert_eq!(mock.process_calls(), 0);
    let job = spawner.take().remove(0);
    job();
    assert_eq!(
        mock.scanned(),
        vec![
            Scan {
                activity: "uploadText".to_owned(),
                data: "hello".to_owned(),
            },
            Scan {
                activity: "downloadText".to_owned(),
                data: "hello".to_owned(),
            },
        ]
    );
    assert_eq!(
        returned.kwargs.get("messages"),
        data(AUDITED_KWARGS).get("messages")
    );
}

#[test]
fn a_scheduled_audit_without_a_trusted_identity_makes_no_calls() {
    let mock = start(Script::passing());
    let engine = engine(&mock.base);
    let spawner = RecordingSpawner::default();
    let _returned = logging_hook(
        &engine,
        call(r#"{"model":"gpt","messages":[{"role":"user","content":"hello"}]}"#),
        &spawner,
    );
    assert_eq!(spawner.scheduled(), 1);
    let job = spawner.take().remove(0);
    job();
    assert_eq!(mock.process_calls(), 0);
}

#[test]
fn the_thread_spawner_runs_the_scheduled_job() {
    let (sender, receiver) = oneshot::channel();
    ThreadAuditSpawner.spawn(Box::new(move || {
        sender.send(()).unwrap();
    }));
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(receiver).unwrap();
}
