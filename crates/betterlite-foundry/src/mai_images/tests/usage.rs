use betterlite_prompts::arguments::json::Object;

use crate::mai_images::error::MaiImageError;
use crate::mai_images::usage::normalize_mai_image_usage;

use super::support::object;

fn normalized(usage: &str) -> Object {
    normalize_mai_image_usage(Some(&object(usage))).unwrap()
}

fn counts(input: i64, image: i64, text: i64, output: i64, total: i64) -> Object {
    object(&format!(
        r#"{{"input_tokens":{input},"input_tokens_details":{{"image_tokens":{image},"text_tokens":{text}}},"output_tokens":{output},"total_tokens":{total}}}"#
    ))
}

pub(super) fn usage_alias_priority() {
    let zeros = counts(0, 0, 0, 0, 0);
    assert_eq!(normalize_mai_image_usage(None).unwrap(), zeros);
    assert_eq!(normalized("{}"), zeros);
    // text: num_input_text_tokens, then details.text_tokens, then input_tokens
    assert_eq!(
        normalized(
            r#"{"num_input_text_tokens":5,"input_tokens_details":{"text_tokens":9},"input_tokens":20}"#
        ),
        counts(20, 0, 5, 0, 20)
    );
    assert_eq!(
        normalized(r#"{"input_tokens_details":{"text_tokens":9,"image_tokens":4}}"#),
        counts(13, 4, 9, 0, 13)
    );
    assert_eq!(normalized(r#"{"input_tokens":7}"#), counts(7, 0, 7, 0, 7));
    assert_eq!(
        normalized(r#"{"num_input_text_tokens":null,"input_tokens":null}"#),
        zeros
    );
    // image: num_input_image_tokens, then details.image_tokens
    assert_eq!(
        normalized(r#"{"num_input_image_tokens":3,"input_tokens_details":{"image_tokens":8}}"#),
        counts(3, 3, 0, 0, 3)
    );
    assert_eq!(
        normalized(r#"{"input_tokens_details":"not an object","num_input_text_tokens":1}"#),
        counts(1, 0, 1, 0, 1)
    );
    // output: output_tokens, num_output_tokens, output_image_tokens
    assert_eq!(
        normalized(r#"{"output_tokens":1,"num_output_tokens":2,"output_image_tokens":3}"#),
        counts(0, 0, 0, 1, 1)
    );
    assert_eq!(
        normalized(r#"{"num_output_tokens":2,"output_image_tokens":3}"#),
        counts(0, 0, 0, 2, 2)
    );
    assert_eq!(
        normalized(r#"{"output_tokens":null,"output_image_tokens":3}"#),
        counts(0, 0, 0, 3, 3)
    );
    // an explicit input or total, zero included, wins over the sum
    assert_eq!(
        normalized(r#"{"num_input_text_tokens":4,"input_tokens":0,"output_tokens":2}"#),
        counts(0, 0, 4, 2, 2)
    );
    assert_eq!(
        normalized(r#"{"num_input_text_tokens":4,"output_tokens":2,"total_tokens":99}"#),
        counts(4, 0, 4, 2, 99)
    );
    assert_eq!(
        normalized(r#"{"num_input_text_tokens":4,"output_tokens":2,"total_tokens":0}"#),
        counts(4, 0, 4, 2, 0)
    );
    assert_eq!(
        normalized(r#"{"num_input_text_tokens":4,"num_input_image_tokens":6,"output_tokens":2}"#),
        counts(10, 6, 4, 2, 12)
    );
    keeps_extra_fields_and_checks_types();
}

fn keeps_extra_fields_and_checks_types() {
    let mut expected = counts(2, 0, 2, 0, 2);
    expected.extend(object(r#"{"cost":0.5,"extra":{"kept":true}}"#));
    assert_eq!(
        normalized(
            r#"{"input_tokens":2,"cost":0.5,"extra":{"kept":true},"input_tokens_details":{"cached_tokens":1}}"#
        ),
        expected
    );
    // an alias behind a winning one is never read
    assert_eq!(
        normalized(r#"{"num_input_text_tokens":5,"input_tokens_details":{"text_tokens":"x"}}"#),
        counts(5, 0, 5, 0, 5)
    );
    let wrong =
        normalize_mai_image_usage(Some(&object(r#"{"num_input_text_tokens":"5"}"#))).unwrap_err();
    assert!(matches!(
        wrong,
        MaiImageError::FieldType {
            field: "num_input_text_tokens"
        }
    ));
    assert_eq!(wrong.status_code(), Some(500));
    let fractional = object(r#"{"input_tokens_details":{"image_tokens":1.5}}"#);
    assert!(matches!(
        normalize_mai_image_usage(Some(&fractional)).unwrap_err(),
        MaiImageError::FieldType {
            field: "input_tokens_details.image_tokens"
        }
    ));
    let input_overflow = object(&format!(
        r#"{{"num_input_text_tokens":{},"num_input_image_tokens":1}}"#,
        i64::MAX
    ));
    assert!(matches!(
        normalize_mai_image_usage(Some(&input_overflow)).unwrap_err(),
        MaiImageError::TokenOverflow {
            field: "input_tokens"
        }
    ));
    let total_overflow = object(&format!(
        r#"{{"input_tokens":{},"output_tokens":1}}"#,
        i64::MAX
    ));
    assert!(matches!(
        normalize_mai_image_usage(Some(&total_overflow)).unwrap_err(),
        MaiImageError::TokenOverflow {
            field: "total_tokens"
        }
    ));
}
