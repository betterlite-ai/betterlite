use std::fs::read_to_string;
use std::sync::Arc;

use tokio::runtime::{Builder, Runtime};
use tracing::subscriber::with_default;

use super::{Gate, TestCapabilities, extra_body, object, value};

fn capture_logs<Run>(run: Run) -> String
where
    Run: FnOnce(),
{
    let log = tempfile::NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().expect("a log writer")))
        .finish();
    with_default(subscriber, run);
    read_to_string(log.path()).expect("the log text")
}

fn test_runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
}

#[test]
fn debug_logs_report_stripped_params_only_when_a_strip_key_is_present() {
    let runtime = test_runtime();
    let with_strip = capture_logs(|| {
        runtime.block_on(async {
            let mut capabilities = TestCapabilities::none();
            let optional = object(r#"{"extra_body":{"stop_token_ids":[1]}}"#);
            extra_body(&mut capabilities, "m", &optional).await;
        });
    });
    assert!(with_strip.contains("does not support NIM/vLLM params"));
    let without_strip = capture_logs(|| {
        runtime.block_on(async {
            let mut capabilities = TestCapabilities::none();
            let optional = object(r#"{"extra_body":{"custom":1}}"#);
            extra_body(&mut capabilities, "m", &optional).await;
        });
    });
    assert!(!without_strip.contains("does not support NIM/vLLM params"));
}

#[test]
fn debug_logs_report_a_dropped_extra_body_response_format_only_when_both_are_set() {
    let runtime = test_runtime();
    let both = capture_logs(|| {
        runtime.block_on(async {
            let mut capabilities = TestCapabilities::none();
            let optional = object(
                r#"{"response_format":{"type":"text"},"extra_body":{"response_format":{"type":"text"}}}"#,
            );
            extra_body(&mut capabilities, "m", &optional).await;
        });
    });
    assert!(both.contains("dropping extra_body.response_format"));
    let optional_only = capture_logs(|| {
        runtime.block_on(async {
            let mut capabilities = TestCapabilities::none();
            let optional =
                object(r#"{"response_format":{"type":"text"},"extra_body":{"custom":1}}"#);
            extra_body(&mut capabilities, "m", &optional).await;
        });
    });
    assert!(!optional_only.contains("dropping extra_body.response_format"));
}

#[test]
fn debug_logs_report_unknown_chat_template_kwargs_keys_only_when_some_are_unknown() {
    let runtime = test_runtime();
    let unknown = capture_logs(|| {
        runtime.block_on(async {
            let mut capabilities = TestCapabilities::none();
            let optional = object(r#"{"extra_body":{"chat_template_kwargs":{"foo":1}}}"#);
            extra_body(&mut capabilities, "m", &optional).await;
        });
    });
    assert!(unknown.contains("does not support chat_template_kwargs keys"));
    let known = capture_logs(|| {
        runtime.block_on(async {
            let mut capabilities = TestCapabilities::with(&[Gate::Reasoning]);
            let optional = object(r#"{"extra_body":{"chat_template_kwargs":{"low_effort":true}}}"#);
            let mapped = extra_body(&mut capabilities, "m", &optional).await;
            assert_eq!(mapped.get("reasoning_effort"), Some(&value(r#""low""#)));
        });
    });
    assert!(!known.contains("does not support chat_template_kwargs keys"));
}
