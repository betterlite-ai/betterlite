#![cfg(test)]

use super::listen_services::Listener;
use super::support::{capture_logs, kwargs, logging_object, messages};

const CATALOG: &[(&str, &str)] = &[(
    "deepgram/streaming/nova-9",
    r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00008}"#,
)];

async fn run_listen(listener: &Listener) {
    let logging_obj = logging_object();
    let frames = messages(r#"[{"type": "Metadata", "duration": 2}]"#);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-9";
    listener
        .listen(&logging_obj, url, &frames, &caller)
        .await
        .expect("the handler runs");
}

#[tokio::test]
async fn deepgram_listen_warns_when_the_registry_lacks_the_base_entry() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);

    run_listen(&Listener::new(&[])).await;

    let contents = logs.contents();
    assert!(contents.contains("no registry entry"));
    assert!(contents.contains("deepgram/streaming/nova-9"));
}

#[tokio::test]
async fn deepgram_listen_stays_quiet_when_the_base_entry_exists() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);

    run_listen(&Listener::new(CATALOG)).await;

    assert!(!logs.contents().contains("no registry entry"));
}
