use rstest::rstest;

use crate::cache_accounting::points::{
    CacheControlInjectionPoint, InjectionIndex, InjectionRole, MessageInjectionPoint,
    ToolConfigInjectionPoint,
};
use crate::cache_accounting::tests::support::json;

fn message_point() -> MessageInjectionPoint {
    MessageInjectionPoint {
        role: None,
        index: None,
        control: None,
        openai_dialect: None,
        external_breakpoints: None,
    }
}

#[test]
fn message_point_serializes_every_declared_field() {
    let point = CacheControlInjectionPoint::Message(MessageInjectionPoint {
        role: Some(InjectionRole::User),
        index: Some(InjectionIndex::Number(1)),
        control: Some(json(r#"{"type": "ephemeral"}"#)),
        openai_dialect: None,
        external_breakpoints: None,
    });
    assert_eq!(
        serde_json::to_string(&point).unwrap(),
        r#"{"location":"message","role":"user","index":1,"control":{"type":"ephemeral"}}"#
    );
}

#[test]
fn tool_config_point_serializes_without_message_fields() {
    let point = CacheControlInjectionPoint::ToolConfig(ToolConfigInjectionPoint {
        control: None,
        openai_dialect: None,
        external_breakpoints: None,
    });
    assert_eq!(
        serde_json::to_string(&point).unwrap(),
        r#"{"location":"tool_config","control":null}"#
    );
}

#[test]
fn a_stamped_point_serializes_the_stamp_keys() {
    let with_count = CacheControlInjectionPoint::Message(message_point()).restamped(None, Some(2));
    let stamped = with_count.restamped(Some(true), None);
    assert_eq!(
        serde_json::to_string(&stamped).unwrap(),
        r#"{"location":"message","role":null,"index":null,"control":null,"_litellm_openai_dialect":true,"_litellm_external_breakpoints":2}"#
    );
}

#[test]
fn points_deserialize_from_the_declared_keys() {
    let parsed: CacheControlInjectionPoint = serde_json::from_str(
        r#"{"location": "message", "role": "system", "index": null, "control": null, "_litellm_openai_dialect": false}"#,
    )
    .unwrap();
    assert_eq!(
        parsed,
        CacheControlInjectionPoint::Message(MessageInjectionPoint {
            role: Some(InjectionRole::System),
            index: None,
            control: None,
            openai_dialect: Some(false),
            external_breakpoints: None,
        })
    );
}

#[rstest]
#[case::number(r#"{"location": "message", "index": 3}"#, InjectionIndex::Number(3))]
#[case::text(r#"{"location": "message", "index": "2"}"#, InjectionIndex::Text("2".to_owned()))]
#[case::negative(r#"{"location": "message", "index": -1}"#, InjectionIndex::Number(-1))]
fn index_deserializes_as_a_number_or_a_string(
    #[case] source: &str,
    #[case] expected: InjectionIndex,
) {
    let parsed: CacheControlInjectionPoint = serde_json::from_str(source).unwrap();
    let CacheControlInjectionPoint::Message(fields) = parsed else {
        panic!("expected a message point");
    };
    assert_eq!(fields.index, Some(expected));
}

#[test]
fn tool_config_points_deserialize_from_their_location() {
    let parsed: CacheControlInjectionPoint =
        serde_json::from_str(r#"{"location": "tool_config", "control": {"type": "ephemeral"}}"#)
            .unwrap();
    assert_eq!(
        parsed,
        CacheControlInjectionPoint::ToolConfig(ToolConfigInjectionPoint {
            control: Some(json(r#"{"type": "ephemeral"}"#)),
            openai_dialect: None,
            external_breakpoints: None,
        })
    );
}

#[test]
fn stamping_returns_a_copy_and_leaves_the_input_alone() {
    let original = CacheControlInjectionPoint::Message(message_point());
    let stamped = original.restamped(Some(true), Some(2));
    assert_ne!(stamped, original);
    assert_eq!(
        original,
        CacheControlInjectionPoint::Message(message_point())
    );
}

#[test]
fn restamping_keeps_a_stamp_the_new_one_does_not_name() {
    let stamped =
        CacheControlInjectionPoint::Message(message_point()).restamped(Some(false), Some(2));
    assert_eq!(
        stamped,
        CacheControlInjectionPoint::Message(MessageInjectionPoint {
            role: None,
            index: None,
            control: None,
            openai_dialect: Some(false),
            external_breakpoints: Some(2),
        })
    );
}

#[test]
fn restamping_replaces_a_stamp_the_point_already_carries() {
    let carried =
        CacheControlInjectionPoint::Message(message_point()).restamped(Some(true), Some(1));
    let stamped = carried.restamped(Some(false), Some(3));
    assert_eq!(
        stamped,
        CacheControlInjectionPoint::Message(MessageInjectionPoint {
            role: None,
            index: None,
            control: None,
            openai_dialect: Some(false),
            external_breakpoints: Some(3),
        })
    );
    assert_eq!(
        carried,
        CacheControlInjectionPoint::Message(MessageInjectionPoint {
            role: None,
            index: None,
            control: None,
            openai_dialect: Some(true),
            external_breakpoints: Some(1),
        })
    );
}
