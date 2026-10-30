#![cfg(test)]
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use std::collections::BTreeMap;
#[test]
fn ignores_unknowns() {
    let values = Vec::from([
        (
            String::from("api_key_hash"),
            LabelDumpValue::String(String::from("hash")),
        ),
        (
            String::from("unknown"),
            LabelDumpValue::String(String::from("ignored")),
        ),
    ]);
    let parsed = UserApiKeyLabelValues::from_entries(&values);
    assert_eq!(parsed.hashed_api_key.as_deref(), Some("hash"));
    assert!(!parsed.model_dump().contains_key("unknown"));
}

#[test]
fn preserves_dump_shape_and_copies() {
    let input = Vec::from([
        (
            "model".to_owned(),
            LabelDumpValue::String("model-1".to_owned()),
        ),
        (
            "stream".to_owned(),
            LabelDumpValue::String("True".to_owned()),
        ),
        (
            "tags".to_owned(),
            LabelDumpValue::Strings(vec!["tag".to_owned()]),
        ),
        (
            "custom_metadata_labels".to_owned(),
            LabelDumpValue::Metadata(BTreeMap::from([("region".to_owned(), "eu".to_owned())])),
        ),
    ]);
    let record = UserApiKeyLabelValues::from_entries(&input);
    let dump = record.model_dump();
    assert_eq!(dump.len(), 33);
    assert_eq!(dump.get("user"), Some(&LabelDumpValue::Null));
    assert!(
        input
            .iter()
            .all(|(name, value)| dump.get(name) == Some(value))
    );
    let mut changed = record.clone();
    changed.tags.clear();
    changed.custom_metadata_labels.clear();
    assert_eq!(record.model_dump(), dump);
    assert_ne!(changed.model_dump(), dump);
}

#[test]
fn defaults_are_null_and_empty() {
    let record = UserApiKeyLabelValues::default();
    let dump = record.model_dump();
    assert_eq!(
        dump.values()
            .filter(|value| **value == LabelDumpValue::Null)
            .count(),
        31
    );
    assert_eq!(dump.get("tags"), Some(&LabelDumpValue::Strings(Vec::new())));
    assert_eq!(
        dump.get("custom_metadata_labels"),
        Some(&LabelDumpValue::Metadata(BTreeMap::new()))
    );
    assert_eq!(format!("{record:?}"), "");
}

#[test]
fn no_op_accepts_metric_calls() {
    use betterlite_prometheus::no_op_metric::{Metric, NoOpMetric};
    let metric = Metric::labels(&NoOpMetric, &[("model", "test")]);
    Metric::inc(&metric, 1);
    Metric::set(&metric, 9);
    Metric::observe(&metric, 2);
    assert_eq!(size_of_val(&metric), 0);
}

#[rstest::rstest]
#[case::boolean(LabelDumpValue::Boolean(true), "True")]
#[case::false_boolean(LabelDumpValue::Boolean(false), "False")]
#[case::integer(LabelDumpValue::Integer(1), "1")]
fn stream_is_stringified(#[case] value: LabelDumpValue, #[case] expected: &str) {
    let record = UserApiKeyLabelValues::from_entries(&[("stream".to_owned(), value)]);
    assert_eq!(record.stream.as_deref(), Some(expected));
}

#[test]
fn aliases_follow_input_order() {
    let entries = vec![
        (
            "hashed_api_key".to_owned(),
            LabelDumpValue::String("original".to_owned()),
        ),
        (
            "api_key_hash".to_owned(),
            LabelDumpValue::String("alias".to_owned()),
        ),
    ];
    assert_eq!(
        UserApiKeyLabelValues::from_entries(&entries)
            .hashed_api_key
            .as_deref(),
        Some("alias")
    );
    let mut reversed = entries;
    reversed.reverse();
    assert_eq!(
        UserApiKeyLabelValues::from_entries(&reversed)
            .hashed_api_key
            .as_deref(),
        Some("original")
    );
    reversed.push(("api_key_hash".to_owned(), LabelDumpValue::Null));
    assert_eq!(
        UserApiKeyLabelValues::from_entries(&reversed).hashed_api_key,
        None
    );
}

#[test]
fn all_string_fields_round_trip() {
    let entries = include_str!("fixtures/labels.txt")
        .lines()
        .filter(|name| *name != "tag" && *name != "callback_name")
        .map(|name| {
            (
                name.to_owned(),
                LabelDumpValue::String(format!("value-{name}")),
            )
        })
        .collect::<Vec<_>>();
    let dump = UserApiKeyLabelValues::from_entries(&entries).model_dump();
    assert_eq!(entries.len(), 31);
    assert!(
        entries
            .iter()
            .all(|(name, value)| dump.get(name) == Some(value))
    );
}
