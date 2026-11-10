#![cfg(test)]

use opentelemetry::Value;

use crate::otel::emitter::message_fitting::fit_indexed_messages;
use crate::otel::mappers::base::AttributeMap;

fn message_attributes(family: &str, index: usize) -> Vec<(String, Value)> {
    vec![
        (
            format!("{family}.{index}.message.role"),
            Value::from("user"),
        ),
        (
            format!("{family}.{index}.message.content"),
            Value::from("hi"),
        ),
    ]
}

fn attributes(entries: Vec<(String, Value)>) -> AttributeMap {
    entries.into_iter().collect()
}

fn three_inputs_and_two_outputs() -> AttributeMap {
    let mut entries = Vec::new();
    for index in 0..3 {
        entries.extend(message_attributes("llm.input_messages", index));
    }
    for index in 0..2 {
        entries.extend(message_attributes("llm.output_messages", index));
    }
    entries.push(("litellm.call_id".to_owned(), Value::from("call-1")));
    attributes(entries)
}

fn has_family(attrs: &AttributeMap, family: &str, index: usize) -> bool {
    attrs.contains_key(&format!("{family}.{index}.message.role"))
}

#[test]
fn a_span_under_its_budget_keeps_every_attribute() {
    let attrs = three_inputs_and_two_outputs();
    let fitted = fit_indexed_messages(attrs.clone(), Some(attrs.len()));
    assert_eq!(fitted, attrs);
}

#[test]
fn an_unbounded_span_keeps_every_attribute() {
    let attrs = three_inputs_and_two_outputs();
    let fitted = fit_indexed_messages(attrs.clone(), None);
    assert_eq!(fitted, attrs);
}

#[test]
fn middle_prompt_turns_are_shed_before_the_opener() {
    let attrs = three_inputs_and_two_outputs();
    let fitted = fit_indexed_messages(attrs.clone(), Some(attrs.len() - 2));
    assert!(!has_family(&fitted, "llm.input_messages", 1));
    assert!(has_family(&fitted, "llm.input_messages", 0));
    assert!(has_family(&fitted, "llm.input_messages", 2));
    assert!(has_family(&fitted, "llm.output_messages", 0));
    assert!(has_family(&fitted, "llm.output_messages", 1));
}

#[test]
fn extra_choices_are_shed_after_the_middle_prompt_turns() {
    let attrs = three_inputs_and_two_outputs();
    let fitted = fit_indexed_messages(attrs.clone(), Some(attrs.len() - 4));
    assert!(!has_family(&fitted, "llm.input_messages", 1));
    assert!(!has_family(&fitted, "llm.output_messages", 1));
    assert!(has_family(&fitted, "llm.output_messages", 0));
    assert!(has_family(&fitted, "llm.input_messages", 2));
}

#[test]
fn the_opener_is_shed_after_the_extra_choices() {
    let attrs = three_inputs_and_two_outputs();
    let fitted = fit_indexed_messages(attrs.clone(), Some(attrs.len() - 6));
    assert!(!has_family(&fitted, "llm.input_messages", 0));
    assert!(has_family(&fitted, "llm.input_messages", 2));
    assert!(has_family(&fitted, "llm.output_messages", 0));
}

#[test]
fn the_first_choice_is_shed_last() {
    let attrs = three_inputs_and_two_outputs();
    let fitted = fit_indexed_messages(attrs, Some(1));
    assert_eq!(fitted.len(), 1);
    assert!(fitted.contains_key("litellm.call_id"));
}

#[test]
fn attributes_that_are_not_indexed_messages_are_never_shed() {
    let attrs = three_inputs_and_two_outputs();
    let fitted = fit_indexed_messages(attrs, Some(0));
    assert_eq!(
        fitted.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["litellm.call_id"]
    );
}

#[test]
fn a_single_prompt_turn_counts_once_in_the_shed_order() {
    let mut entries = message_attributes("llm.input_messages", 0);
    entries.push(("litellm.call_id".to_owned(), Value::from("call-1")));
    let attrs = attributes(entries);
    let fitted = fit_indexed_messages(attrs, Some(1));
    assert_eq!(
        fitted.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["litellm.call_id"]
    );
}

#[test]
fn message_keys_with_a_non_numeric_index_are_not_grouped() {
    let attrs = attributes(vec![
        (
            "llm.input_messages.first.message.role".to_owned(),
            Value::from("user"),
        ),
        ("litellm.call_id".to_owned(), Value::from("call-1")),
    ]);
    let fitted = fit_indexed_messages(attrs.clone(), Some(1));
    assert_eq!(fitted, attrs);
}
