use std::iter::once;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::spend_sanitize::REDACTED_BY_LITELM;
use crate::spend_sanitize::request_body::sanitize_request_body;
use crate::spend_sanitize::value::{NodeId, SpendMapping, SpendValue, to_json};

use super::support::{lookup, number, parse, spend_number};

#[test]
fn numbers_every_node_so_equal_siblings_stay_distinct() {
    let source = parse(r#"{"a":{"api_key":"sk-one"},"b":{"api_key":"sk-two"}}"#);
    let SpendValue::Mapping(mapping) = SpendValue::from(&source) else {
        panic!("a JSON object is a mapping")
    };
    let sanitized = sanitize_request_body(&mapping, 2048);
    assert_eq!(
        lookup(&sanitized, "a.api_key"),
        &Json::Text(REDACTED_BY_LITELM.to_owned())
    );
    assert_eq!(
        lookup(&sanitized, "b.api_key"),
        &Json::Text(REDACTED_BY_LITELM.to_owned())
    );
}

#[test]
fn converts_every_json_shape() {
    let source = parse(r#"{"list":[1,"two",null],"object":{"flag":true},"n":17}"#);
    let converted = SpendValue::from(&source);
    assert_eq!(to_json(&converted), source);
}

#[test]
fn renders_models_and_objects_by_their_fields() {
    let model = SpendValue::Model(SpendMapping {
        node: NodeId::from(1_u64),
        entries: once(("a".to_owned(), spend_number(1))).collect(),
    });
    let expected = Json::Object(once(("a".to_owned(), number(1))).collect::<IndexMap<_, _>>());
    assert_eq!(to_json(&model), expected);
    let object = SpendValue::Object(SpendMapping {
        node: NodeId::from(2_u64),
        entries: IndexMap::new(),
    });
    assert_eq!(to_json(&object), Json::Object(IndexMap::new()));
}
