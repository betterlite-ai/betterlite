#![cfg(test)]

use opentelemetry::Value;

use crate::otel::mappers::base::AttributeMap;
use crate::otel::mappers::openinference::fit_indexed_messages;

fn core_attributes(attributes: &mut AttributeMap) {
    attributes.insert("openinference.span.kind".to_owned(), Value::from("LLM"));
    attributes.insert("llm.model_name".to_owned(), Value::from("gpt-4o"));
    attributes.insert("llm.provider".to_owned(), Value::from("openai"));
    attributes.insert("llm.token_count.prompt".to_owned(), Value::I64(10));
    attributes.insert("llm.token_count.completion".to_owned(), Value::I64(4));
    attributes.insert("llm.token_count.total".to_owned(), Value::I64(14));
}

fn with_message_family(attributes: &mut AttributeMap, prefix: &str, role: &str, count: usize) {
    for index in 0..count {
        attributes.insert(
            format!("{prefix}.{index}.message.role"),
            Value::from(role.to_owned()),
        );
        attributes.insert(
            format!("{prefix}.{index}.message.content"),
            Value::from(format!("{role} {index}")),
        );
    }
}

fn with_messages(inputs: usize, outputs: usize) -> AttributeMap {
    let mut attributes = AttributeMap::new();
    core_attributes(&mut attributes);
    with_message_family(&mut attributes, "llm.input_messages", "user", inputs);
    with_message_family(&mut attributes, "llm.output_messages", "assistant", outputs);
    attributes
}

#[test]
fn message_shedding_without_a_budget_keeps_every_key() {
    let attributes = with_messages(2, 2);
    let fitted = fit_indexed_messages(attributes.clone(), None);
    assert_eq!(fitted, attributes);
}

#[test]
fn message_shedding_within_the_budget_keeps_every_key() {
    let attributes = with_messages(2, 2);
    let fitted = fit_indexed_messages(attributes.clone(), Some(attributes.len()));
    assert_eq!(fitted, attributes);
}

#[test]
fn message_shedding_drops_the_middle_prompts_first() {
    let fitted = fit_indexed_messages(with_messages(3, 0), Some(10));
    assert_eq!(fitted.len(), 10);
    assert!(!fitted.contains_key("llm.input_messages.1.message.role"));
    assert!(!fitted.contains_key("llm.input_messages.1.message.content"));
    assert!(fitted.contains_key("llm.input_messages.0.message.content"));
    assert!(fitted.contains_key("llm.input_messages.2.message.content"));
}

#[test]
fn message_shedding_drops_the_newest_extra_choices_before_the_first() {
    let fitted = fit_indexed_messages(with_messages(1, 3), Some(12));
    assert_eq!(fitted.len(), 12);
    assert!(!fitted.contains_key("llm.output_messages.2.message.role"));
    assert!(fitted.contains_key("llm.output_messages.1.message.content"));
    assert!(fitted.contains_key("llm.output_messages.0.message.role"));
    assert!(fitted.contains_key("llm.input_messages.0.message.content"));
}

#[test]
fn message_shedding_keeps_the_opener_newest_prompt_and_first_choice() {
    let fitted = fit_indexed_messages(with_messages(3, 3), Some(12));
    assert_eq!(fitted.len(), 12);
    assert!(!fitted.contains_key("llm.input_messages.1.message.role"));
    assert!(!fitted.contains_key("llm.output_messages.2.message.role"));
    assert!(!fitted.contains_key("llm.output_messages.1.message.role"));
    assert!(fitted.contains_key("llm.input_messages.0.message.content"));
    assert!(fitted.contains_key("llm.input_messages.2.message.content"));
    assert!(fitted.contains_key("llm.output_messages.0.message.content"));
    assert!(fitted.contains_key("openinference.span.kind"));
}

#[test]
fn message_shedding_sheds_every_message_when_the_core_needs_the_room() {
    let fitted = fit_indexed_messages(with_messages(2, 2), Some(2));
    assert_eq!(fitted.len(), 6);
    assert!(fitted.contains_key("openinference.span.kind"));
    assert!(fitted.contains_key("llm.token_count.total"));
    assert!(!fitted.contains_key("llm.input_messages.0.message.role"));
    assert!(!fitted.contains_key("llm.output_messages.0.message.role"));
}

#[test]
fn message_shedding_ignores_keys_outside_the_message_families() {
    let mut attributes = with_messages(2, 2);
    attributes.insert("llm.input_messages".to_owned(), Value::from("summary"));
    let fitted = fit_indexed_messages(attributes, Some(10));
    assert!(fitted.contains_key("llm.input_messages"));
}
