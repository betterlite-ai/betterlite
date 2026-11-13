#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::shared::{analyzer, settings};
use crate::presidio_analysis::RequestOverrides;

#[rstest]
#[case::text_and_language_only(
    None,
    Vec::new(),
    None,
    BTreeMap::new(),
    r#"{"text":"hello","language":"en"}"#
)]
#[case::entities_come_from_the_pii_config(
    None,
    vec!["PERSON".to_owned(), "EMAIL_ADDRESS".to_owned()],
    None,
    BTreeMap::new(),
    r#"{"text":"hello","language":"en","entities":["PERSON","EMAIL_ADDRESS"]}"#
)]
#[case::ad_hoc_recognizers_are_sent_when_set(
    None,
    Vec::new(),
    Some(vec![GuardrailValue::Text("titles".to_owned())]),
    BTreeMap::new(),
    r#"{"text":"hello","language":"en","ad_hoc_recognizers":["titles"]}"#
)]
#[case::empty_ad_hoc_recognizers_are_still_sent(
    None,
    Vec::new(),
    Some(Vec::new()),
    BTreeMap::new(),
    r#"{"text":"hello","language":"en","ad_hoc_recognizers":[]}"#
)]
#[case::request_language_wins(
    Some("de"),
    Vec::new(),
    None,
    BTreeMap::new(),
    r#"{"text":"hello","language":"de"}"#
)]
#[case::dynamic_params_are_merged(
    None,
    Vec::new(),
    None,
    BTreeMap::from([("score_threshold".to_owned(), GuardrailValue::Decimal(0.5))]),
    r#"{"text":"hello","language":"en","score_threshold":0.5}"#
)]
#[case::dynamic_params_replace_typed_fields(
    Some("de"),
    vec!["PERSON".to_owned()],
    None,
    BTreeMap::from([
        ("language".to_owned(), GuardrailValue::Text("fr".to_owned())),
        ("entities".to_owned(), GuardrailValue::Array(vec![GuardrailValue::Text("PHONE_NUMBER".to_owned())])),
    ]),
    r#"{"text":"hello","entities":["PHONE_NUMBER"],"language":"fr"}"#
)]
#[case::a_null_dynamic_param_is_sent_as_null(
    None,
    Vec::new(),
    None,
    BTreeMap::from([("language".to_owned(), GuardrailValue::Null)]),
    r#"{"text":"hello","language":null}"#
)]
fn analyze_payload_has_the_analyzer_request_shape(
    #[case] request_language: Option<&str>,
    #[case] pii_entities: Vec<String>,
    #[case] ad_hoc_recognizers: Option<Vec<GuardrailValue>>,
    #[case] dynamic_params: BTreeMap<String, GuardrailValue>,
    #[case] expected: &str,
) {
    let mut configured = settings("http://unused");
    configured.pii_entities = pii_entities;
    configured.ad_hoc_recognizers = ad_hoc_recognizers;
    let request = RequestOverrides {
        language: request_language,
        dynamic_params: &dynamic_params,
    };
    let payload = analyzer(configured).analyze_request_payload("hello", &request);
    assert_eq!(serde_json::to_string(&payload).unwrap(), expected);
}
