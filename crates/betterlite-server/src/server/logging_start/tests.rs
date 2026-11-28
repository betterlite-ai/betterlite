#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_logging::logging_streams::assembler::ChatStreamAssembler;
use betterlite_logging::registry::CallbackRegistry;

use super::start_logging;

const CONFIG: &str = r#"
model_list:
  - {model_name: g, litellm_params: {model: openai/m, api_key: k}}
litellm_settings:
  turn_off_message_logging: true
  standard_logging_payload_excluded_fields: ["messages", "response"]
  log_client_error_tracebacks: true
"#;

#[tokio::test]
async fn start_logging_reads_the_handler_settings_from_the_config() {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment,
    }
    .load(CONFIG)
    .unwrap();
    let wiring = start_logging(
        &config,
        CallbackRegistry::new(16),
        Arc::new(ChatStreamAssembler),
    )
    .expect("the logging parts start");
    assert_eq!(
        wiring.success.excluded_fields,
        vec!["messages".to_owned(), "response".to_owned()]
    );
    assert!(wiring.success.privacy.turn_off_message_logging);
    assert!(!wiring.success.privacy.redact_user_api_key_info);
    let known = wiring.success.known_callback.as_ref();
    assert!(known("langfuse"));
    assert!(!known("my_custom_callback"));
    wiring.pipeline.stop().await;
}
