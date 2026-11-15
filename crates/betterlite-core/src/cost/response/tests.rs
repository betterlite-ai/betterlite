#![cfg(test)]

use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use rstest::rstest;

use crate::cost::completion::error::{ChargeInputError, CompletionCostError};
use crate::cost::completion::tests::rig::{
    Stored, args, chat_usage, chat_view, dec, dictionary_view, map, respond, rig,
};

const GPT: &[(&str, &str)] = &[
    ("input_cost_per_token", "0.001"),
    ("output_cost_per_token", "0.002"),
];

fn header(value: &str) -> String {
    format!(r#"{{"additional_headers":{{"llm_provider-x-litellm-response-cost":{value}}}}}"#)
}

#[rstest]
#[case::text("\"0.5\"", "0.5")]
#[case::padded_text("\" 0.5 \"", "0.5")]
#[case::number("0.25", "0.25")]
#[case::zero("0", "0")]
#[case::true_flag("true", "1")]
#[tokio::test]
async fn a_provider_stated_cost_wins_over_the_calculation(
    #[case] value: &str,
    #[case] expected: &str,
) {
    let usage = chat_usage(10, 5);
    let hidden = map(&header(value));
    let view = chat_view(&usage, Some("gpt-4o"), Some(&hidden));
    let charge = respond(
        &rig(&[("gpt-4o", GPT)]),
        &args(Some(&view)),
        None,
        None::<&mut Stored>,
    )
    .await
    .unwrap();
    assert_eq!(charge, dec(expected));
}

#[tokio::test]
async fn a_provider_stated_cost_wins_under_custom_pricing() {
    let usage = chat_usage(10, 5);
    let hidden = map(&header("\"0.5\""));
    let view = chat_view(&usage, Some("gpt-4o"), Some(&hidden));
    let mut call = args(Some(&view));
    call.billing.custom_pricing = Some(true);
    let charge = respond(&rig(&[("gpt-4o", GPT)]), &call, None, None::<&mut Stored>)
        .await
        .unwrap();
    assert_eq!(charge, dec("0.5"));
}

#[rstest]
#[case::hit(Some(true), "0")]
#[case::miss(Some(false), "0.5")]
#[case::unknown(None, "0.5")]
#[tokio::test]
async fn only_a_cache_hit_makes_the_call_free(
    #[case] cache_hit: Option<bool>,
    #[case] expected: &str,
) {
    let usage = chat_usage(10, 5);
    let hidden = map(&header("\"0.5\""));
    let view = chat_view(&usage, Some("gpt-4o"), Some(&hidden));
    let mut stored = Stored::default();
    let charge = respond(
        &rig(&[("gpt-4o", GPT)]),
        &args(Some(&view)),
        cache_hit,
        Some(&mut stored),
    )
    .await
    .unwrap();
    assert_eq!(charge, dec(expected));
    assert!(stored.0.is_none());
}

#[rstest]
#[case::absent_header(r#"{"additional_headers":{}}"#)]
#[case::null_header(r#"{"additional_headers":{"llm_provider-x-litellm-response-cost":null}}"#)]
#[case::no_headers("{}")]
#[case::null_headers(r#"{"additional_headers":null}"#)]
#[tokio::test]
async fn a_missing_or_null_header_falls_through_to_the_calculation(#[case] hidden: &str) {
    let usage = chat_usage(10, 5);
    let headers = map(hidden);
    let view = chat_view(&usage, Some("gpt-4o"), Some(&headers));
    let charge = respond(
        &rig(&[("gpt-4o", GPT)]),
        &args(Some(&view)),
        None,
        None::<&mut Stored>,
    )
    .await
    .unwrap();
    assert_eq!(charge, dec("0.02"));
}

#[tokio::test]
async fn a_dictionary_response_has_no_hidden_attributes() {
    let usage = chat_usage(10, 5);
    let view = dictionary_view(&usage);
    let charge = respond(
        &rig(&[("gpt-4o", GPT)]),
        &args(Some(&view)),
        None,
        None::<&mut Stored>,
    )
    .await
    .unwrap();
    assert_eq!(charge, dec("0.02"));
}

#[rstest]
#[case::list("[1]")]
#[case::object("{}")]
#[tokio::test]
async fn a_header_that_is_not_a_number_is_a_typed_error(#[case] value: &str) {
    let usage = chat_usage(10, 5);
    let hidden = map(&header(value));
    let view = chat_view(&usage, Some("gpt-4o"), Some(&hidden));
    let error = respond(
        &rig(&[("gpt-4o", GPT)]),
        &args(Some(&view)),
        None,
        None::<&mut Stored>,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        CompletionCostError::Input(ChargeInputError::CostHeader(_))
    ));
}

#[tokio::test]
async fn a_header_text_that_is_not_a_decimal_keeps_the_parse_cause() {
    let usage = chat_usage(10, 5);
    let hidden = map(&header("\" free \""));
    let view = chat_view(&usage, Some("gpt-4o"), Some(&hidden));
    let error = respond(
        &rig(&[("gpt-4o", GPT)]),
        &args(Some(&view)),
        None,
        None::<&mut Stored>,
    )
    .await
    .unwrap_err();
    let CompletionCostError::Input(ChargeInputError::CostHeaderValue { header, source }) = error
    else {
        panic!("the header parse failure is typed and carries its text");
    };
    assert_eq!(header, "free");
    assert!(matches!(source, DecimalLexemeError::Decimal(_)));
}

#[tokio::test]
async fn the_calculation_errors_propagate() {
    let error = respond(&rig(&[]), &args(None), None, None::<&mut Stored>)
        .await
        .unwrap_err();
    assert!(matches!(error, CompletionCostError::Prepare(_)));
}
