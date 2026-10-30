use crate::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use std::collections::BTreeMap;
#[test]
fn ignores_unknowns() {
    let values = BTreeMap::from([
        (
            String::from("api_key_hash"),
            LabelDumpValue::String(String::from("hash")),
        ),
        (
            String::from("unknown"),
            LabelDumpValue::String(String::from("ignored")),
        ),
    ]);
    let parsed = UserApiKeyLabelValues::from_map(&values);
    assert_eq!(parsed.hashed_api_key.as_deref(), Some("hash"));
    assert!(!parsed.model_dump().contains_key("unknown"));
}
