#![cfg(test)]

use crate::langfuse_handler::LangFuseHandler;
use crate::langfuse_logger::langfuse::{
    LangFuseLogger, LangfuseLoggedEvent, LangfuseLoggingConfig,
};

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn type_definitions() {
    let event = LangfuseLoggedEvent {
        trace_id: Some("trace".to_owned()),
        generation_id: None,
    };
    let encoded = serde_json::to_string(&event).expect("the logged event serializes");
    assert_eq!(encoded, r#"{"trace_id":"trace","generation_id":null}"#);
    let decoded: LangfuseLoggedEvent =
        serde_json::from_str(&encoded).expect("the logged event deserializes");
    assert_eq!(decoded, event);
    let config = LangfuseLoggingConfig::default();
    assert!(config.langfuse_secret.is_none());
    assert!(config.langfuse_public_key.is_none());
    assert!(config.langfuse_host.is_none());
    assert!(config.langfuse_environment.is_none());
    assert_eq!(std::mem::size_of::<LangFuseHandler>(), 0);
    assert_send_sync::<LangFuseLogger>();
    assert_send_sync::<LangFuseHandler>();
}
