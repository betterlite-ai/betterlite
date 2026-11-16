#![cfg(test)]

use super::listen_services::listen;
use super::support::{Registry, capture_logs, kwargs, logging_object, lookup, messages, registry};

const CATALOG: &[(&str, &str)] = &[(
    "deepgram/streaming/nova-9",
    r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00008}"#,
)];

async fn run_listen(registry: &Registry) {
    let logging_obj = logging_object();
    let frames = messages(r#"[{"type": "Metadata", "duration": 2}]"#);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-9";
    listen(registry, &logging_obj, url, &frames, &caller)
        .await
        .expect("the handler runs");
}

#[tokio::test]
async fn deepgram_listen_warns_when_the_registry_lacks_the_base_entry() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);

    run_listen(&registry(&[])).await;

    let contents = logs.contents();
    assert!(contents.contains("no registry entry"));
    assert!(contents.contains("deepgram/streaming/nova-9"));
}

#[tokio::test]
async fn deepgram_listen_stays_quiet_when_the_base_entry_exists() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);

    run_listen(&registry(CATALOG)).await;

    assert!(!logs.contents().contains("no registry entry"));
}

#[tokio::test]
async fn deepgram_listen_stays_quiet_when_only_the_base_rate_is_unreadable() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    let unreadable = Registry {
        catalog: registry(CATALOG).catalog,
        lookup: lookup(&[(
            "deepgram/streaming/nova-9",
            r#"{"litellm_provider": "deepgram", "input_cost_per_second": [1]}"#,
        )]),
    };

    run_listen(&unreadable).await;

    assert!(!logs.contents().contains("no registry entry"));
}
