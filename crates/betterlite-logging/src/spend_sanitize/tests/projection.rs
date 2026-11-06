use std::iter::once;

use betterlite_models::decisions::Json;

use crate::spend_sanitize::projection::convert_to_json_serializable;
use crate::spend_sanitize::value::{NodeId, SpendMapping, SpendValue};

use super::support::{lookup, mapping, number, parse, sequence, spend_number, text};

#[test]
fn emits_a_circular_reference_for_a_cycle_on_the_current_path() {
    let cyclic = mapping(1, vec![("self", mapping(1, Vec::new()))]);
    let projected = convert_to_json_serializable(&cyclic);
    assert_eq!(
        lookup(&projected, "self"),
        &Json::Text("<circular_reference>".to_owned())
    );
}

#[test]
fn emits_a_circular_reference_for_a_list_cycle() {
    let cyclic = sequence(1, vec![sequence(1, Vec::new())]);
    let projected = convert_to_json_serializable(&cyclic);
    assert_eq!(
        lookup(&projected, "0"),
        &Json::Text("<circular_reference>".to_owned())
    );
}

#[test]
fn expands_a_repeated_mapping_again_after_unwinding() {
    let shared = mapping(2, vec![("leaf", text("value"))]);
    let body = mapping(1, vec![("first", shared.clone()), ("second", shared)]);
    let projected = convert_to_json_serializable(&body);
    assert_eq!(
        lookup(&projected, "first.leaf"),
        &Json::Text("value".to_owned())
    );
    assert_eq!(
        lookup(&projected, "second.leaf"),
        &Json::Text("value".to_owned())
    );
}

fn nested(wrappers: u32) -> SpendValue {
    (0..wrappers).fold(mapping(0, vec![("leaf", text("value"))]), |inner, level| {
        mapping(u64::from(level).saturating_add(10), vec![("n", inner)])
    })
}

#[test]
fn emits_max_depth_exceeded_at_the_depth_boundary() {
    let inside = convert_to_json_serializable(&nested(18));
    assert_eq!(
        lookup(&inside, &format!("{}leaf", "n.".repeat(18))),
        &Json::Text("value".to_owned())
    );
    let at_boundary = convert_to_json_serializable(&nested(19));
    assert_eq!(
        lookup(&at_boundary, &format!("{}leaf", "n.".repeat(19))),
        &Json::Text("<max_depth_exceeded>".to_owned())
    );
    let beyond = convert_to_json_serializable(&nested(20));
    assert_eq!(
        lookup(&beyond, &"n.".repeat(20)),
        &Json::Text("<max_depth_exceeded>".to_owned())
    );
}

#[test]
fn projects_models_and_objects_through_their_fields() {
    let model = SpendValue::Model(SpendMapping {
        node: NodeId::from(1_u64),
        entries: once(("a".to_owned(), spend_number(1))).collect(),
    });
    let projected_model = convert_to_json_serializable(&model);
    assert_eq!(lookup(&projected_model, "a"), &number(1));

    let object = SpendValue::Object(SpendMapping {
        node: NodeId::from(2_u64),
        entries: once(("b".to_owned(), sequence(3, vec![spend_number(2)]))).collect(),
    });
    let projected_object = convert_to_json_serializable(&object);
    assert_eq!(lookup(&projected_object, "b.0"), &number(2));
}

#[test]
fn passes_primitives_through_unchanged() {
    let source = parse(r#"{"s":"text","n":17,"flag":false,"none":null}"#);
    let projected = convert_to_json_serializable(&SpendValue::from(&source));
    assert_eq!(projected, source);
}
