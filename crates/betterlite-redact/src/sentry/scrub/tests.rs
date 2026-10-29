#![cfg(test)]

use super::super::settings::build_sentry_init_options;
use super::{EventValue, MAX_SCRUB_DEPTH, scrub_event};
use std::collections::BTreeMap;

const DEFAULT_CUSTOM_KEY_MINIMUM: usize = 16;
const DEFAULT_GENERATED_KEY_BYTES: usize = 16;

fn object(pairs: &[(&str, EventValue)]) -> EventValue {
    EventValue::Object(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn text(value: &str) -> EventValue {
    EventValue::String(value.to_owned())
}

fn nested_object(depth: usize, leaf: EventValue) -> EventValue {
    let mut current = leaf;
    for _ in 0..depth {
        let mut map = BTreeMap::new();
        map.insert("n".to_owned(), current);
        current = EventValue::Object(map);
    }
    current
}

fn innermost_by<'value>(
    value: &'value EventValue,
    step: impl Fn(&'value EventValue) -> Option<&'value EventValue>,
) -> &'value EventValue {
    let mut current = value;
    while let Some(next) = step(current) {
        current = next;
    }
    current
}

fn innermost_object(value: &EventValue) -> &EventValue {
    innermost_by(value, |current| {
        let EventValue::Object(map) = current else {
            return None;
        };
        map.get("n")
    })
}

fn nested_array(depth: usize, leaf: EventValue) -> EventValue {
    let mut current = leaf;
    for _ in 0..depth {
        current = EventValue::Array(vec![current]);
    }
    current
}

#[test]
fn sentry_scrubbing_behavior_tests_behavior() {
    let at_limit = nested_object(MAX_SCRUB_DEPTH, text("api_key='secret'"));
    let scrubbed_at_limit = scrub_event(
        &at_limit,
        false,
        DEFAULT_CUSTOM_KEY_MINIMUM,
        DEFAULT_GENERATED_KEY_BYTES,
    )
    .unwrap();
    assert_eq!(
        innermost_object(&scrubbed_at_limit),
        &text("api_key='[Filtered]'")
    );

    let beyond_limit = nested_object(MAX_SCRUB_DEPTH.saturating_add(1), text("harmless"));
    let scrubbed_beyond_limit = scrub_event(
        &beyond_limit,
        false,
        DEFAULT_CUSTOM_KEY_MINIMUM,
        DEFAULT_GENERATED_KEY_BYTES,
    )
    .unwrap();
    assert_eq!(
        innermost_object(&scrubbed_beyond_limit),
        &text("[Filtered]")
    );
}

#[test]
fn a_lowered_custom_key_minimum_scrubs_shorter_keys() {
    let event = text("key is sk-abcde here");

    let default_lengths = scrub_event(
        &event,
        false,
        DEFAULT_CUSTOM_KEY_MINIMUM,
        DEFAULT_GENERATED_KEY_BYTES,
    )
    .unwrap();
    assert_eq!(default_lengths, event);

    let mut env = BTreeMap::new();
    env.insert("MINIMUM_CUSTOM_KEY_LENGTH".to_owned(), "8".to_owned());
    let options = build_sentry_init_options(&env).unwrap();
    let lowered_minimum = scrub_event(
        &event,
        false,
        options.custom_key_minimum,
        options.generated_key_bytes,
    )
    .unwrap();
    assert_eq!(lowered_minimum, text("key is [Filtered] here"));
}

#[test]
fn array_nesting_also_counts_toward_scrub_depth() {
    let nested = nested_array(MAX_SCRUB_DEPTH.saturating_add(1), text("harmless"));
    let scrubbed = scrub_event(
        &nested,
        false,
        DEFAULT_CUSTOM_KEY_MINIMUM,
        DEFAULT_GENERATED_KEY_BYTES,
    )
    .unwrap();
    let innermost = innermost_by(&scrubbed, |value| {
        let EventValue::Array(items) = value else {
            return None;
        };
        items.first()
    });
    assert_eq!(innermost, &text("[Filtered]"));
}

fn wrap_frame_in_stacktrace(frame: EventValue) -> EventValue {
    object(&[(
        "exception",
        object(&[(
            "values",
            EventValue::Array(vec![object(&[(
                "stacktrace",
                object(&[("frames", EventValue::Array(vec![frame]))]),
            )])]),
        )]),
    )])
}

fn first_frame(event: &EventValue) -> &BTreeMap<String, EventValue> {
    let EventValue::Object(root) = event else {
        panic!("expected an object root");
    };
    let EventValue::Object(exception) = &root["exception"] else {
        panic!("expected exception object");
    };
    let EventValue::Array(values) = &exception["values"] else {
        panic!("expected values array");
    };
    let EventValue::Object(first_value) = &values[0] else {
        panic!("expected a value object");
    };
    let EventValue::Object(stacktrace) = &first_value["stacktrace"] else {
        panic!("expected stacktrace object");
    };
    let EventValue::Array(frames) = &stacktrace["frames"] else {
        panic!("expected frames array");
    };
    let EventValue::Object(frame) = &frames[0] else {
        panic!("expected a frame object");
    };
    frame
}

#[test]
fn source_context_keys_survive_inside_known_stack_frame_shapes() {
    let built_frame = object(&[
        (
            "pre_context",
            EventValue::Array(vec![text("user@example.com")]),
        ),
        ("context_line", text("token=abc")),
        ("post_context", EventValue::Array(vec![text("other")])),
        ("vars", object(&[("details", text("api_key='abc'"))])),
    ]);
    let event = wrap_frame_in_stacktrace(built_frame);

    let scrubbed = scrub_event(
        &event,
        false,
        DEFAULT_CUSTOM_KEY_MINIMUM,
        DEFAULT_GENERATED_KEY_BYTES,
    )
    .unwrap();
    let frame = first_frame(&scrubbed);

    assert_eq!(
        frame["pre_context"],
        EventValue::Array(vec![text("user@example.com")])
    );
    assert_eq!(frame["context_line"], text("token=abc"));
    assert_eq!(
        frame["post_context"],
        EventValue::Array(vec![text("other")])
    );
    assert_eq!(
        frame["vars"],
        object(&[("details", text("api_key='[Filtered]'"))])
    );
}

#[test]
fn source_context_keys_are_scrubbed_outside_a_known_stack_frame_shape() {
    let event = object(&[("context_line", text("token=abc"))]);
    let scrubbed = scrub_event(
        &event,
        false,
        DEFAULT_CUSTOM_KEY_MINIMUM,
        DEFAULT_GENERATED_KEY_BYTES,
    )
    .unwrap();
    let EventValue::Object(root) = &scrubbed else {
        panic!("expected an object root");
    };
    assert_eq!(root["context_line"], text("token='[Filtered]'"));
}
