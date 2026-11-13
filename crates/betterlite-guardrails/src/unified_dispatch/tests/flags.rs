#[test]
fn flag_default_applies_when_nothing_is_set() {
    let default = GuardrailValue::Boolean(false);
    assert_eq!(
        resolve_streaming_flag(
            None,
            None,
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &default
        ),
        default
    );
}

#[test]
fn flag_attribute_beats_the_default() {
    let attribute = GuardrailValue::Boolean(true);
    assert_eq!(
        resolve_streaming_flag(
            Some(&attribute),
            None,
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        attribute
    );
}

#[test]
fn flag_config_beats_the_attribute() {
    let config = BTreeMap::from([(
        "streaming_end_of_stream_only".to_owned(),
        GuardrailValue::Boolean(false),
    )]);
    assert_eq!(
        resolve_streaming_flag(
            Some(&GuardrailValue::Boolean(true)),
            Some(&config),
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        GuardrailValue::Boolean(false)
    );
}

#[test]
fn flag_optional_params_beat_the_config() {
    let config = BTreeMap::from([(
        "streaming_sampling_rate".to_owned(),
        GuardrailValue::SignedInteger(5),
    )]);
    let optional_params = BTreeMap::from([(
        "streaming_sampling_rate".to_owned(),
        GuardrailValue::SignedInteger(9),
    )]);
    assert_eq!(
        resolve_streaming_flag(
            None,
            Some(&config),
            &optional_params,
            "streaming_sampling_rate",
            &GuardrailValue::SignedInteger(1),
        ),
        GuardrailValue::SignedInteger(9)
    );
}

#[test]
fn flag_a_present_null_in_the_config_wins() {
    let config = BTreeMap::from([(
        "streaming_end_of_stream_only".to_owned(),
        GuardrailValue::Null,
    )]);
    assert_eq!(
        resolve_streaming_flag(
            Some(&GuardrailValue::Boolean(true)),
            Some(&config),
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        GuardrailValue::Null
    );
}

#[test]
fn flag_a_present_null_in_the_optional_params_wins() {
    let optional_params = BTreeMap::from([(
        "streaming_end_of_stream_only".to_owned(),
        GuardrailValue::Null,
    )]);
    assert_eq!(
        resolve_streaming_flag(
            Some(&GuardrailValue::Boolean(true)),
            None,
            &optional_params,
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        GuardrailValue::Null
    );
}

#[test]
fn flag_config_without_the_name_keeps_the_attribute() {
    let config = BTreeMap::from([("other_key".to_owned(), GuardrailValue::Boolean(false))]);
    assert_eq!(
        resolve_streaming_flag(
            Some(&GuardrailValue::Boolean(true)),
            Some(&config),
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        GuardrailValue::Boolean(true)
    );
}

fn scan_key(texts: &[&str]) -> StreamingScanKey {
    StreamingScanKey {
        texts: texts.iter().map(|value| (*value).to_owned()).collect(),
        tool_calls: Vec::new(),
        stream_ended: false,
        tool_calls_in_flight: false,
    }
}

#[test]
fn flag_no_scan_key_is_not_redundant() {
    assert!(!is_redundant_scan(None, None));
    assert!(!is_redundant_scan(None, Some(&scan_key(&["a"]))));
}

#[test]
fn flag_equal_scan_keys_are_redundant() {
    let key = scan_key(&["a"]);
    let mut last = scan_key(&["a"]);
    last.stream_ended = true;
    last.tool_calls_in_flight = true;
    assert!(is_redundant_scan(Some(&key), Some(&last)));
}

#[test]
fn flag_an_empty_scan_key_is_redundant() {
    let key = scan_key(&["", ""]);
    assert!(is_redundant_scan(Some(&key), None));
}

#[test]
fn flag_different_scan_keys_are_not_redundant() {
    let key = scan_key(&["a"]);
    let last = scan_key(&["b"]);
    assert!(!is_redundant_scan(Some(&key), Some(&last)));
    assert!(!is_redundant_scan(Some(&key), None));
}

#[test]
fn flag_a_key_with_tool_calls_has_something_to_scan() {
    let mut key = scan_key(&[""]);
    key.tool_calls = vec!["call_1".to_owned()];
    assert!(!is_redundant_scan(Some(&key), None));
}
