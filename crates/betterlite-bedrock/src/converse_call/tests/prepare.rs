#![cfg(test)]

use super::{
    ExtractInput, Prepared, extract, pop_bool, pop_text, signed_header_sources, signing_region,
    stored_region, transform_headers,
};
use crate::converse_call::ConverseEnvironment;
use crate::converse_call::target::PreparedTarget;
use crate::converse_call::tests::call;
use crate::model_routes::{ParamMap, ParamValue};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_models::controls::control_options::ControlOptions;
use indexmap::IndexMap;
use secrecy::SecretString;

#[rstest::rstest]
#[case::absent("{}", None)]
#[case::null("{\"stream\":null}", None)]
#[case::false_value("{\"stream\":false}", Some(false))]
#[case::true_value("{\"stream\":true}", Some(true))]
fn bool_options(#[case] source: &str, #[case] expected: Option<bool>) {
    let mut params: ParamMap = serde_json::from_str(source).unwrap();
    assert_eq!(pop_bool(&mut params, "stream").unwrap(), expected);
    assert!(params.is_empty());
}

#[rstest::rstest]
#[case::absent("{}", None)]
#[case::null("{\"model_id\":null}", None)]
#[case::empty("{\"model_id\":\"\"}", Some(""))]
#[case::explicit("{\"model_id\":\"a/b\"}", Some("a/b"))]
fn text_options(#[case] source: &str, #[case] expected: Option<&str>) {
    let mut params: ParamMap = serde_json::from_str(source).unwrap();
    assert_eq!(
        pop_text(&mut params, "model_id").unwrap().as_deref(),
        expected
    );
    assert!(params.is_empty());
}

#[test]
fn invalid_options_name_the_field_without_its_secret_value() {
    let mut params: ParamMap =
        serde_json::from_str(r#"{"stream":"fixture-secret","model_id":false}"#).unwrap();
    assert_eq!(
        pop_bool(&mut params, "stream").unwrap_err().to_string(),
        "the stream parameter has the wrong type"
    );
    assert_eq!(
        pop_text(&mut params, "model_id").unwrap_err().to_string(),
        "the model_id parameter has the wrong type"
    );
}

fn prepared(params: &mut ParamMap, model: &str) -> Prepared {
    extract(
        &mut ExtractInput {
            params,
            model,
            extra_headers: None,
            control: ControlOptions::default(),
            api_key: None,
            environment: ConverseEnvironment::default(),
        },
        &BetaMatrix(IndexMap::default()),
    )
    .unwrap()
}

#[rstest::rstest]
#[case::absent(ParamMap::new(), ParamValue::Text("us-east-1".to_owned()))]
#[case::present_null(
    ParamMap::from([("aws_region_name".to_owned(), ParamValue::Null)]),
    ParamValue::Null
)]
#[case::present_text(
    ParamMap::from([("aws_region_name".to_owned(), ParamValue::Text("eu-west-1".to_owned()))]),
    ParamValue::Text("eu-west-1".to_owned())
)]
fn model_region_preserves_explicit_presence(
    #[case] mut params: ParamMap,
    #[case] expected: ParamValue,
) {
    let output = prepared(&mut params, "bedrock/us-east-1/nova/model");
    assert_eq!(output.model_id, "model");
    assert_eq!(
        params,
        ParamMap::from([("aws_region_name".to_owned(), expected)])
    );
}

#[test]
fn routing_pops_keep_json_mode_and_chunk_size_only_for_streaming() {
    let mut params: ParamMap = serde_json::from_str(
        r#"{"stream":true,"model_id":"a/b","fake_stream":true,"json_mode":true}"#,
    )
    .unwrap();
    let output = prepared(&mut params, "model");
    assert_eq!(output.stream, Some(true));
    assert!(output.fake_stream);
    assert_eq!(output.model_id, "a%2Fb");
    assert_eq!(params.len(), 1);
    assert_eq!(params.get("json_mode"), Some(&ParamValue::Bool(true)));
}

#[rstest::rstest]
#[case::sync(false, "unsupported", "us-east-1")]
#[case::asynchronous(true, "", "eu-west-1")]
fn beta_and_signing_sources(#[case] asynchronous: bool, #[case] beta: &str, #[case] region: &str) {
    let secret = SecretString::from("fixture-token");
    let mut request = call(&secret, &secret);
    request.acompletion = asynchronous;
    request.litellm_params = serde_json::from_str(r#"{"aws_region_name":"eu-west-1"}"#).unwrap();
    let extras = vec![
        ("anthropic-beta".to_owned(), "unsupported".to_owned()),
        ("Content-Type".to_owned(), "custom".to_owned()),
    ];
    request.extra_headers = Some(&extras);
    let output = extract(
        &mut ExtractInput {
            params: &mut ParamMap::new(),
            model: "model",
            extra_headers: Some(&extras),
            control: ControlOptions::default(),
            api_key: Some(&secret),
            environment: ConverseEnvironment::default(),
        },
        &BetaMatrix(IndexMap::default()),
    )
    .unwrap();
    let transformed = transform_headers(&request, &output).unwrap();
    assert_eq!(transformed.contains_key("anthropic-beta"), !beta.is_empty());
    let sources = signed_header_sources(&request, &output);
    assert!(
        sources
            .headers
            .contains(&("Content-Type".to_owned(), Some("custom".to_owned())))
    );
    assert_eq!(
        sources
            .extra
            .iter()
            .any(|(name, _)| name == "anthropic-beta"),
        !asynchronous
    );
    let target = PreparedTarget {
        region: "us-east-1".to_owned(),
        credentials: None,
        url: String::new(),
    };
    assert_eq!(signing_region(&request, &target), region);
}

#[rstest::rstest]
#[case::absent("{}", "us-west-2")]
#[case::empty("{\"aws_region_name\":\"\"}", "us-west-2")]
#[case::present("{\"aws_region_name\":\"eu-west-1\"}", "eu-west-1")]
fn async_signing_region(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        stored_region(&serde_json::from_str(source).unwrap()),
        expected
    );
}

#[rstest::rstest]
#[case::stream(true, Some(19))]
#[case::buffered(false, None)]
fn control_chunk_size_is_conditional(#[case] stream: bool, #[case] expected: Option<u64>) {
    let mut params = ParamMap::from([("stream".to_owned(), ParamValue::Bool(stream))]);
    let value = extract(
        &mut ExtractInput {
            params: &mut params,
            model: "model",
            extra_headers: None,
            control: ControlOptions::new(Some(19)).unwrap(),
            api_key: None,
            environment: ConverseEnvironment::default(),
        },
        &BetaMatrix(IndexMap::default()),
    )
    .unwrap();
    assert_eq!(value.stream_chunk_size, expected);
    assert!(params.is_empty());
}
