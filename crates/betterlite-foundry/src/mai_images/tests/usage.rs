use betterlite_prompts::arguments::json::Object;

use crate::mai_images::usage::normalize_mai_image_usage;

use super::support::{kind, object, variant};

// Each usage with the counts [input, image, text, output, total] the reply must carry
const ALIAS_ROWS: [(&str, [i64; 5]); 20] = [
    (
        r#"{"num_input_text_tokens":null,"input_tokens":null}"#,
        [0; 5],
    ),
    (
        r#"{"num_input_text_tokens":5,"input_tokens_details":{"text_tokens":9},"input_tokens":20}"#,
        [20, 0, 5, 0, 20],
    ),
    (
        r#"{"input_tokens_details":{"text_tokens":9,"image_tokens":4}}"#,
        [13, 4, 9, 0, 13],
    ),
    (r#"{"input_tokens":7}"#, [7, 0, 7, 0, 7]),
    (
        r#"{"num_input_image_tokens":3,"input_tokens_details":{"image_tokens":8}}"#,
        [3, 3, 0, 0, 3],
    ),
    (
        r#"{"input_tokens_details":"not an object","num_input_text_tokens":1}"#,
        [1, 0, 1, 0, 1],
    ),
    (
        r#"{"output_tokens":1,"num_output_tokens":2,"output_image_tokens":3}"#,
        [0, 0, 0, 1, 1],
    ),
    (
        r#"{"num_output_tokens":2,"output_image_tokens":3}"#,
        [0, 0, 0, 2, 2],
    ),
    (
        r#"{"output_tokens":null,"output_image_tokens":3}"#,
        [0, 0, 0, 3, 3],
    ),
    (
        r#"{"num_input_text_tokens":0,"input_tokens_details":{"text_tokens":9}}"#,
        [0; 5],
    ),
    (
        r#"{"num_input_image_tokens":0,"input_tokens_details":{"image_tokens":8}}"#,
        [0; 5],
    ),
    (r#"{"output_tokens":0,"num_output_tokens":2}"#, [0; 5]),
    (
        r#"{"num_input_text_tokens":4,"input_tokens":0,"output_tokens":2}"#,
        [0, 0, 4, 2, 2],
    ),
    (
        r#"{"num_input_text_tokens":4,"output_tokens":2,"total_tokens":99}"#,
        [4, 0, 4, 2, 99],
    ),
    (
        r#"{"num_input_text_tokens":4,"output_tokens":2,"total_tokens":0}"#,
        [4, 0, 4, 2, 0],
    ),
    (
        r#"{"num_input_text_tokens":4,"num_input_image_tokens":6,"output_tokens":2}"#,
        [10, 6, 4, 2, 12],
    ),
    (
        r#"{"num_input_text_tokens":5,"input_tokens_details":{"text_tokens":"x"}}"#,
        [5, 0, 5, 0, 5],
    ),
    (
        r#"{"input_tokens":2,"cost":0.5,"extra":{"kept":true},"input_tokens_details":{"cached_tokens":1}}"#,
        [2, 0, 2, 0, 2],
    ),
    (r#"{"input_tokens":0,"total_tokens":5}"#, [0, 0, 0, 0, 5]),
    (
        r#"{"output_tokens":4,"total_tokens":null}"#,
        [0, 0, 0, 4, 4],
    ),
];

fn normalized(usage: &str) -> Object {
    normalize_mai_image_usage(Some(&object(usage))).unwrap()
}

fn counts(expected_counts: [i64; 5]) -> Object {
    let [input, image, text, output, total] = expected_counts;
    object(&format!(
        r#"{{"input_tokens":{input},"input_tokens_details":{{"image_tokens":{image},"text_tokens":{text}}},"output_tokens":{output},"total_tokens":{total}}}"#
    ))
}

pub(super) fn usage_alias_priority() {
    let zeros = counts([0; 5]);
    assert_eq!(normalize_mai_image_usage(None).unwrap(), zeros);
    assert_eq!(normalized("{}"), zeros);
    for (usage, expected_counts) in ALIAS_ROWS {
        // the reply keeps every field it came with, the four counts and the details replaced
        let mut expected = object(usage);
        expected.extend(counts(expected_counts));
        assert_eq!(normalized(usage), expected, "{usage}");
    }
    wrong_types_and_overflow();
}

fn wrong_types_and_overflow() {
    let usage = |text: &str| kind(normalize_mai_image_usage(Some(&object(text))));
    assert_eq!(usage(r#"{"num_input_text_tokens":"5"}"#), "field_type");
    assert_eq!(usage(r#"{"output_tokens":true}"#), "field_type");
    assert_eq!(
        usage(r#"{"input_tokens_details":{"image_tokens":1.5}}"#),
        "field_type"
    );
    assert_eq!(
        usage(r#"{"input_tokens":9223372036854775808}"#),
        "field_type"
    );
    let max = i64::MAX;
    let input_overflow = format!(r#"{{"num_input_text_tokens":{max},"num_input_image_tokens":1}}"#);
    assert_eq!(usage(&input_overflow), "token_overflow");
    let total_overflow = format!(r#"{{"input_tokens":{max},"output_tokens":1}}"#);
    assert_eq!(usage(&total_overflow), "token_overflow");
    let wrong =
        normalize_mai_image_usage(Some(&object(r#"{"num_input_text_tokens":"5"}"#))).unwrap_err();
    assert_eq!(variant(&wrong), "field_type");
    assert_eq!(wrong.status_code(), Some(500));
    assert_eq!(
        wrong.to_string(),
        "the Azure AI MAI image field num_input_text_tokens has the wrong type"
    );
}
