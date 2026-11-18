#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{AppliedInputType, ModerationResponse, ModerationResponseError};

const NAMES: [&str; 13] = [
    "harassment",
    "harassment/threatening",
    "hate",
    "hate/threatening",
    "illicit",
    "illicit/violent",
    "self-harm",
    "self-harm/instructions",
    "self-harm/intent",
    "sexual",
    "sexual/minors",
    "violence",
    "violence/graphic",
];

fn every_category(value: Value) -> Map<String, Value> {
    NAMES
        .iter()
        .map(|name| ((*name).to_owned(), value.clone()))
        .collect()
}

fn result_with(scores: Map<String, Value>) -> Value {
    json!({
        "categories": every_category(json!(false)),
        "category_applied_input_types": every_category(json!(["text"])),
        "category_scores": scores,
        "flagged": false,
    })
}

fn body_with(result: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "modr-1",
        "model": "omni-moderation-latest",
        "results": [result],
    }))
    .expect("provider body")
}

fn body() -> Vec<u8> {
    body_with(result_with(every_category(json!(0.0001))))
}

#[rstest]
#[case::full_body(body_with(result_with(every_category(json!(0.0001)))))]
fn moderations_moderation_tests_behavior(#[case] body: Vec<u8>) {
    let response = ModerationResponse::parse(&body).expect("moderation response");
    assert_eq!(response.id, "modr-1");
    assert_eq!(response.model, "omni-moderation-latest");
    let result = response.results.first().expect("result");
    assert!(!result.categories.harassment);
    assert!(!result.categories.harassment_threatening);
    assert_eq!(result.categories.illicit, Some(false));
    assert_eq!(
        result.category_applied_input_types.self_harm,
        vec![AppliedInputType::Text]
    );
    assert_eq!(
        result.category_scores.violence_graphic.0,
        parse_decimal_lexeme("0.0001").expect("score")
    );
    assert!(!result.flagged);
}

#[test]
fn the_client_body_uses_the_names_litellm_dumps() {
    let response = ModerationResponse::parse(&body()).expect("moderation response");
    let text = serde_json::to_string(&response).expect("client body");
    // the SDK model dumps its field names, so a slash key the provider sent fills a field and is not dumped again
    assert!(text.contains(r#""harassment_threatening":false"#), "{text}");
    assert!(!text.contains("harassment/threatening"), "{text}");
    assert!(text.contains(r#""self_harm":["text"]"#), "{text}");
    assert!(!text.contains("self-harm"), "{text}");
    assert!(text.contains(r#""violence_graphic":0.0001"#), "{text}");
    assert!(!text.contains("violence/graphic"), "{text}");
    assert!(text.contains(r#""illicit":false"#), "{text}");
    assert!(text.contains(r#""illicit_violent":false"#), "{text}");
}

#[test]
fn keeps_provider_extras_at_every_level() {
    let mut result = result_with(every_category(json!(0.5)));
    result
        .as_object_mut()
        .expect("result object")
        .insert(String::from("provider_note"), json!({"kept": true}));
    let mut categories = every_category(json!(false));
    categories.insert(String::from("new_category"), json!(true));
    result
        .as_object_mut()
        .expect("result object")
        .insert(String::from("categories"), Value::Object(categories));
    let mut document =
        serde_json::from_slice::<Value>(&body_with(result)).expect("provider document");
    document
        .as_object_mut()
        .expect("document object")
        .insert(String::from("provider_note"), json!("top"));
    let bytes = serde_json::to_vec(&document).expect("provider body");
    let client =
        serde_json::to_value(ModerationResponse::parse(&bytes).expect("moderation response"))
            .expect("client body");
    assert_eq!(client["provider_note"], json!("top"));
    assert_eq!(client["results"][0]["provider_note"], json!({"kept": true}));
    assert_eq!(
        client["results"][0]["categories"]["new_category"],
        json!(true)
    );
}

#[rstest]
#[case::smallest("1e-28", "0.0000000000000000000000000001")]
#[case::unrepresentable("1e-29", "")]
#[case::too_large("1e29", "")]
fn a_score_is_kept_exactly_or_refused(#[case] score: &str, #[case] expected: &str) {
    let mut scores = every_category(json!(0.5));
    scores.insert(
        String::from("hate"),
        serde_json::from_str(score).expect("score lexeme"),
    );
    let body = body_with(result_with(scores));
    if expected.is_empty() {
        let error = ModerationResponse::parse(&body).expect_err("out of range score");
        assert!(
            matches!(error, ModerationResponseError::Decimal { category, .. } if category == "hate"),
            "{error}"
        );
        return;
    }
    let response = ModerationResponse::parse(&body).expect("moderation response");
    let text = serde_json::to_string(&response).expect("client body");
    assert!(
        text.contains(&format!(r#""hate":{expected}"#)),
        "the score must not be rounded: {text}"
    );
    assert!(
        response.results[0].category_scores.hate.0 == parse_decimal_lexeme(score).expect("score")
    );
}

#[rstest]
#[case::missing_result_field("flagged")]
#[case::missing_categories("categories")]
#[case::missing_scores("category_scores")]
fn rejects_a_result_missing_a_field_litellm_requires(#[case] field: &str) {
    let mut result = result_with(every_category(json!(0.5)));
    result.as_object_mut().expect("result object").remove(field);
    assert!(matches!(
        ModerationResponse::parse(&body_with(result)),
        Err(ModerationResponseError::Decode { .. })
    ));
}

#[rstest]
#[case::missing_category("harassment")]
#[case::missing_score("hate")]
#[case::missing_applied_type("sexual")]
fn rejects_a_named_category_missing_from_one_map(#[case] name: &str) {
    let mut result = result_with(every_category(json!(0.5)));
    for field in [
        "categories",
        "category_applied_input_types",
        "category_scores",
    ] {
        result[field]
            .as_object_mut()
            .expect("category map")
            .remove(name);
    }
    assert!(matches!(
        ModerationResponse::parse(&body_with(result)),
        Err(ModerationResponseError::Decode { .. })
    ));
}

#[rstest]
#[case::score_is_text("category_scores", json!("high"))]
#[case::flagged_is_text("flagged", json!("no"))]
#[case::unknown_applied_type("category_applied_input_types", json!(["audio"]))]
fn rejects_a_wrongly_typed_result_field(#[case] field: &str, #[case] value: Value) {
    let mut result = result_with(every_category(json!(0.5)));
    if field == "category_applied_input_types" {
        result[field] = every_category(value).into();
    } else {
        result[field] = value;
    }
    assert!(matches!(
        ModerationResponse::parse(&body_with(result)),
        Err(ModerationResponseError::Decode { .. })
    ));
}

#[rstest]
#[case::malformed(b"{".to_vec())]
#[case::results_not_an_array(br#"{"id":"modr-1","model":"m","results":{}}"#.to_vec())]
#[case::missing_model(br#"{"id":"modr-1","results":[]}"#.to_vec())]
fn rejects_a_body_that_is_not_a_moderation_response(#[case] body: Vec<u8>) {
    assert!(matches!(
        ModerationResponse::parse(&body),
        Err(ModerationResponseError::Decode { .. })
    ));
}

#[test]
fn an_empty_result_list_is_allowed() {
    let response = ModerationResponse::parse(br#"{"id":"modr-1","model":"m","results":[]}"#)
        .expect("moderation response");
    assert!(response.results.is_empty());
}

#[test]
fn an_optional_category_stays_absent_as_null() {
    let mut categories = every_category(json!(false));
    categories.remove("illicit");
    categories.remove("illicit/violent");
    let mut result = result_with(every_category(json!(0.5)));
    result["categories"] = Value::Object(categories);
    let response = ModerationResponse::parse(&body_with(result)).expect("moderation response");
    assert_eq!(response.results[0].categories.illicit, None);
    assert_eq!(response.results[0].categories.illicit_violent, None);
    let text = serde_json::to_string(&response).expect("client body");
    assert!(text.contains(r#""illicit":null"#), "{text}");
    assert!(text.contains(r#""illicit_violent":null"#), "{text}");
}

#[rstest]
fn replays_the_moderation_response_corpus(
    #[files("fuzz/corpus/moderation_response/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _response = ModerationResponse::parse(&bytes);
}
