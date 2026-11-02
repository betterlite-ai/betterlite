#[cfg(test)]
use rstest::rstest;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::test_object;

#[cfg(test)]
#[rstest]
#[case::interception_marker(test_object([("_code_interpreter_interception_active", json!(true)), ("other", json!("kept"))]))]
#[case::websearch_marker(test_object([("_websearch_interception_active", json!(true)), ("other", json!("kept"))]))]
#[case::agentic_loop_marker(test_object([("_agentic_loop_depth", json!(2_i64)), ("other", json!("kept"))]))]
#[case::max_agentic_loops(test_object([("max_agentic_loops", json!(4_i64)), ("other", json!("kept"))]))]
fn strip_removes_interception_markers_and_keeps_other_keys(#[case] metadata: Value) {
    let mut kwargs = Map::new();
    kwargs.insert("litellm_metadata".to_owned(), metadata);
    strip(&mut kwargs);
    let remaining = kwargs.get("litellm_metadata").unwrap().as_object().unwrap();
    assert_eq!(remaining.keys().collect::<Vec<_>>(), ["other"]);
}

#[cfg(test)]
#[test]
fn strip_drops_the_metadata_key_entirely_when_nothing_is_left() {
    let mut kwargs = Map::new();
    kwargs.insert(
        "litellm_metadata".to_owned(),
        test_object([("_code_interpreter_interception_active", json!(true))]),
    );
    strip(&mut kwargs);
    assert!(!kwargs.contains_key("litellm_metadata"));
}

#[cfg(test)]
#[test]
fn strip_does_nothing_when_litellm_metadata_is_absent() {
    let mut kwargs = Map::new();
    kwargs.insert("other".to_owned(), json!("value"));
    strip(&mut kwargs);
    assert_eq!(kwargs.get("litellm_metadata"), None);
}

#[cfg(test)]
#[test]
fn write_adds_every_present_marker_and_keeps_existing_metadata() {
    let mut kwargs = Map::new();
    kwargs.insert(
        "litellm_metadata".to_owned(),
        test_object([("existing", json!("value"))]),
    );
    kwargs.insert(INTERCEPTION_ACTIVE_KEY.to_owned(), json!(true));
    kwargs.insert(SANDBOX_KEY.to_owned(), json!("sandbox-1"));
    write(&mut kwargs);
    let metadata = kwargs.get("litellm_metadata").unwrap().as_object().unwrap();
    assert_eq!(metadata.get("existing"), Some(&json!("value")));
    assert_eq!(metadata.get(INTERCEPTION_ACTIVE_KEY), Some(&json!(true)));
    assert_eq!(metadata.get(SANDBOX_KEY), Some(&json!("sandbox-1")));
    assert_eq!(metadata.get(SESSION_SCOPED_KEY), None);
}

#[cfg(test)]
#[test]
fn write_creates_metadata_when_none_existed() {
    let mut kwargs = Map::new();
    kwargs.insert(CONVERTED_STREAM_KEY.to_owned(), json!(true));
    write(&mut kwargs);
    let metadata = kwargs.get("litellm_metadata").unwrap().as_object().unwrap();
    assert_eq!(metadata.get(CONVERTED_STREAM_KEY), Some(&json!(true)));
}

#[cfg(test)]
#[rstest]
#[case::present(test_object([("sandbox_key", json!("abc"))]), Some("abc".to_owned()))]
#[case::absent(json!({}), None)]
#[case::non_string(test_object([("sandbox_key", json!(5_i64))]), None)]
fn sandbox_key_reads_the_sandbox_key_field(
    #[case] metadata: Value,
    #[case] expected: Option<String>,
) {
    assert_eq!(sandbox_key(metadata.as_object().unwrap()), expected);
}
