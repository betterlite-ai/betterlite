fn info_from(text: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(text).unwrap()
}

#[test]
fn json_dict_accepts_objects_and_object_text() {
    let entries = BTreeMap::from([(String::from("a"), json("\"b\""))]);
    let object = JsonValue::Object(entries.clone());
    assert_eq!(deserialize_json_dict(Some(&object)), Some(entries));
    let text = JsonValue::String(String::from(r#"{"a":"b"}"#));
    let expected = BTreeMap::from([(String::from("a"), json("\"b\""))]);
    assert_eq!(deserialize_json_dict(Some(&text)), Some(expected));
    let empty_text = JsonValue::String(String::from("{}"));
    assert_eq!(
        deserialize_json_dict(Some(&empty_text)),
        Some(BTreeMap::new())
    );
}

#[test]
fn json_dict_gives_none_for_empty_and_other_shapes() {
    assert!(deserialize_json_dict(None).is_none());
    let empty = JsonValue::Object(BTreeMap::new());
    assert!(deserialize_json_dict(Some(&empty)).is_none());
    assert!(deserialize_json_dict(Some(&json("null"))).is_none());
    assert!(deserialize_json_dict(Some(&json("true"))).is_none());
    assert!(deserialize_json_dict(Some(&json("1"))).is_none());
    assert!(deserialize_json_dict(Some(&json("[1]"))).is_none());
    assert!(deserialize_json_dict(Some(&json(r#""[1]""#))).is_none());
    assert!(deserialize_json_dict(Some(&json(r#""not json""#))).is_none());
}

#[test]
fn json_list_accepts_lists_and_list_text() {
    let entries = vec![json(r#"{"name":"test-name"}"#)];
    let listed = JsonValue::Array(entries.clone());
    assert_eq!(deserialize_json_list(Some(&listed)), Some(entries));
    let text = json(r#""[1,2]""#);
    assert_eq!(
        deserialize_json_list(Some(&text)),
        Some(vec![json("1"), json("2")])
    );
    let empty_text = json(r#""[]""#);
    assert_eq!(deserialize_json_list(Some(&empty_text)), Some(Vec::new()));
}

#[test]
fn json_list_gives_none_for_empty_and_other_shapes() {
    assert!(deserialize_json_list(None).is_none());
    let empty = JsonValue::Array(Vec::new());
    assert!(deserialize_json_list(Some(&empty)).is_none());
    assert!(deserialize_json_list(Some(&json("null"))).is_none());
    assert!(deserialize_json_list(Some(&json("1"))).is_none());
    assert!(deserialize_json_list(Some(&json("{}"))).is_none());
    assert!(deserialize_json_list(Some(&json(r#""{}""#))).is_none());
    assert!(deserialize_json_list(Some(&json(r#""not json""#))).is_none());
}

#[test]
fn json_cost_info_normalizes_numbers_and_strings() {
    let mut info = info_from(
        r#"{"server_name":"test-name","mcp_server_cost_info":{"default_cost_per_query":"0.5","tool_name_to_cost_per_query":{"tool_a":0.25,"tool_b":"1.5"}}}"#,
    );
    normalize_cost_info(&mut info);
    let text = serde_json::to_string(&info).unwrap();
    assert_eq!(
        text,
        r#"{"mcp_server_cost_info":{"default_cost_per_query":0.5,"tool_name_to_cost_per_query":{"tool_a":0.25,"tool_b":1.5}},"server_name":"test-name"}"#
    );
}

#[test]
fn json_cost_info_drops_uncoercible_values_with_a_warning() {
    let (captured, _guard) = capture();
    let mut info = info_from(
        r#"{"server_name":"test-name","mcp_server_cost_info":{"default_cost_per_query":"free","tool_name_to_cost_per_query":{"tool_a":true}}}"#,
    );
    normalize_cost_info(&mut info);
    let text = serde_json::to_string(&info).unwrap();
    assert_eq!(
        text,
        r#"{"mcp_server_cost_info":{"tool_name_to_cost_per_query":{}},"server_name":"test-name"}"#
    );
    let logged = captured.text();
    assert!(logged.contains("has non-numeric default_cost_per_query 'free'; ignoring it"));
    assert!(logged.contains("has non-numeric cost True for tool 'tool_a'; ignoring it"));
}

#[test]
fn json_cost_info_keeps_nulls_and_non_objects() {
    let mut with_null = info_from(r#"{"mcp_server_cost_info":{"default_cost_per_query":null}}"#);
    normalize_cost_info(&mut with_null);
    let text = serde_json::to_string(&with_null).unwrap();
    assert_eq!(
        text,
        r#"{"mcp_server_cost_info":{"default_cost_per_query":null}}"#
    );
    let mut raw = info_from(r#"{"mcp_server_cost_info":"raw"}"#);
    normalize_cost_info(&mut raw);
    let raw_text = serde_json::to_string(&raw).unwrap();
    assert_eq!(raw_text, r#"{"mcp_server_cost_info":"raw"}"#);
}
