use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::support::{data, header, headers};

#[rstest]
#[case::an_unsigned_integer(1)]
fn guardrail_metadata_cases(#[case] number: u64) {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "semantic-similarity".to_owned(),
        GuardrailValue::UnsignedInteger(number),
    );
    let mut request_data = data("{}");
    request_data.insert("metadata".to_owned(), GuardrailValue::Object(metadata));
    assert_eq!(
        header(&headers(&request_data), "x-litellm-semantic-similarity"),
        Some("1")
    );
}
