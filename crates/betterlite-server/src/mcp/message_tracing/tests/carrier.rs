use std::collections::BTreeMap;

use rmcp::model::MetaObject;

use crate::mcp::message_tracing::meta_carrier;

fn meta(json: &str) -> MetaObject {
    serde_json::from_str(json).expect("metadata should decode")
}

#[test]
fn carrier_whitelist() {
    let carrier = meta_carrier(Some(&meta(
        r#"{"traceparent":"00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01","tracestate":"vendor=value","baggage":"litellm.team.id=spoofed","litellm.team.id":"team-1","progressToken":7}"#,
    )));
    let expected = BTreeMap::from([
        (
            "traceparent".to_owned(),
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_owned(),
        ),
        ("tracestate".to_owned(), "vendor=value".to_owned()),
    ]);
    assert_eq!(carrier, Some(expected));
}

#[test]
fn absent_meta() {
    assert_eq!(meta_carrier(None), None);
    assert_eq!(meta_carrier(Some(&MetaObject::default())), None);
}

#[test]
fn baggage_is_not_a_carrier_key() {
    assert_eq!(
        meta_carrier(Some(&meta(
            r#"{"baggage":"litellm.team.id=spoofed","litellm.team.id":"team-1"}"#
        ))),
        None
    );
}

#[test]
fn empty_string_is_still_a_string() {
    let carrier = meta_carrier(Some(&meta(r#"{"traceparent":"","tracestate":""}"#)));
    let expected = BTreeMap::from([
        ("traceparent".to_owned(), String::new()),
        ("tracestate".to_owned(), String::new()),
    ]);
    assert_eq!(carrier, Some(expected));
}

#[test]
fn non_string_values_are_ignored() {
    let carrier = meta_carrier(Some(&meta(r#"{"traceparent":7,"tracestate":""}"#)));
    assert_eq!(
        carrier,
        Some(BTreeMap::from([("tracestate".to_owned(), String::new())]))
    );
    assert_eq!(
        meta_carrier(Some(&meta(
            r#"{"traceparent":null,"tracestate":["vendor=value"]}"#
        ))),
        None
    );
}
