#![cfg(test)]

use betterlite_core::api::media_relay_logging::deepgram::DeepgramListenRequest;

use super::deepgram::deepgram_listen_passthrough_handler;
use super::support::{Registry, capture_logs, kwargs, logging_object, lookup, messages, registry};

const CATALOG: &[(&str, &str)] = &[(
    "deepgram/streaming/nova-9",
    r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00008}"#,
)];

fn listen(registry: &Registry) {
    let logging_obj = logging_object();
    let frames = messages(r#"[{"type": "Metadata", "duration": 2}]"#);
    let caller = kwargs(&[]);
    let request = DeepgramListenRequest {
        upstream_url: "https://api.deepgram.com/v1/listen?model=nova-9",
        messages: &frames,
        kwargs: &caller,
        catalog: &registry.catalog,
        lookup: &registry.lookup,
    };
    deepgram_listen_passthrough_handler(&logging_obj, &request).expect("the handler runs");
}

#[test]
fn deepgram_listen_warns_when_the_registry_lacks_the_base_entry() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);

    listen(&registry(&[]));

    let contents = logs.contents();
    assert!(contents.contains("no registry entry"));
    assert!(contents.contains("deepgram/streaming/nova-9"));
}

#[test]
fn deepgram_listen_stays_quiet_when_the_base_entry_exists() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);

    listen(&registry(CATALOG));

    assert!(!logs.contents().contains("no registry entry"));
}

#[test]
fn deepgram_listen_stays_quiet_when_only_the_base_rate_is_unreadable() {
    let (logs, _guard) = capture_logs(tracing::Level::WARN);
    let unreadable = Registry {
        catalog: registry(CATALOG).catalog,
        lookup: lookup(&[(
            "deepgram/streaming/nova-9",
            r#"{"litellm_provider": "deepgram", "input_cost_per_second": [1]}"#,
        )]),
    };

    listen(&unreadable);

    assert!(!logs.contents().contains("no registry entry"));
}
