#![cfg(test)]

use super::*;

#[test]
fn sanitize_preserves_boolean_primitives() {
    let values = BTreeMap::from([
        ("enabled".to_owned(), OtelModelValue::Bool(true)),
        ("disabled".to_owned(), OtelModelValue::Bool(false)),
    ]);
    assert_eq!(
        sanitize_event_metadata(&values),
        BTreeMap::from([
            ("enabled".to_owned(), "true".to_owned()),
            ("disabled".to_owned(), "false".to_owned()),
        ])
    );
}
#[test]
fn sanitize_filters_secrets_and_bounds_values() {
    let source = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(
        r#"{"authorization":"x","safe":"ok","function_duration":12,"function_name":"drop"}"#,
    )
    .unwrap();
    assert_eq!(
        sanitize_event_metadata(&source).get("safe"),
        Some(&"ok".to_owned())
    );
    assert_eq!(
        sanitize_event_metadata(&source).get("function_duration"),
        Some(&"12".to_owned())
    );
}

#[test]
fn sanitize_bounds_unicode_and_count() {
    let long = "\u{1f642}".repeat(1100);
    let mut values = (0_u32..40)
        .map(|index| {
            (
                format!("key{index:02}"),
                OtelModelValue::String(long.clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    values.insert(
        "Authorization".to_owned(),
        OtelModelValue::String("hidden".to_owned()),
    );
    values.insert(
        "function_args".to_owned(),
        OtelModelValue::String("hidden".to_owned()),
    );
    let result = sanitize_event_metadata(&values);
    assert_eq!(result.len(), 32);
    assert_eq!(result.get("key00"), Some(&"\u{1f642}".repeat(1024)));
    assert!(!result.contains_key("Authorization"));
    assert!(!result.contains_key("function_args"));
    assert!(!result.contains_key("key32"));
}

#[test]
fn sanitize_bounds_large_integer_values() {
    let number = serde_json::from_str(&"9".repeat(1100)).unwrap();
    let values = BTreeMap::from([("count".to_owned(), OtelModelValue::Number(number))]);
    assert_eq!(
        sanitize_event_metadata(&values).get("count"),
        Some(&"9".repeat(1024))
    );
}

#[rstest::rstest]
#[case::key("API_KEY")]
#[case::token("access_token")]
#[case::secret("client_secret")]
#[case::password("Password")]
#[case::cookie("Cookies")]
#[case::headers("response_headers")]
#[case::hidden("_hidden_params")]
fn sensitive_substrings_are_filtered(#[case] key: &str) {
    let values = BTreeMap::from([(key.to_owned(), OtelModelValue::String("hidden".to_owned()))]);
    assert!(sanitize_event_metadata(&values).is_empty());
}
