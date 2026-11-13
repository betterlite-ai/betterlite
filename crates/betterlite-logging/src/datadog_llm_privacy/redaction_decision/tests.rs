#![cfg(test)]

use crate::datadog_llm_privacy::redaction_decision::REDACTS_MESSAGES_ITSELF;

#[test]
fn the_logger_redacts_messages_itself() {
    assert_eq!(REDACTS_MESSAGES_ITSELF.to_string(), "true");
}
