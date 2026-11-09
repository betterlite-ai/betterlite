use http::Method;
use rstest::rstest;

use crate::json::{Json, Object};
use crate::ocr_config::{OcrProviderConfig as _, PreparedOcrRequest};
use crate::ocr_error::OcrError;
use crate::ocr_http::OcrClient;
use crate::textract_ocr::analyze::TextractAnalyzeDocumentConfig;
use crate::textract_ocr::detect::TextractDetectTextConfig;

use super::support::{
    NoHooks, body_bytes, body_json, header, json, region_params, request, signing_client,
    with_secret,
};

#[tokio::test]
async fn a_detect_request_is_a_signed_post_to_the_regional_endpoint() {
    let (_log, client) = signing_client();
    let request = request("detect-document-text", None, region_params("us-east-1"));

    let outbound = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .expect("the detect request prepares");

    assert_eq!(outbound.method, Method::POST);
    assert_eq!(
        outbound.url.as_str(),
        "https://textract.us-east-1.amazonaws.com/"
    );
    assert_eq!(
        header(&outbound, "x-amz-target").as_deref(),
        Some("Textract.DetectDocumentText")
    );
    assert_eq!(
        header(&outbound, "content-type").as_deref(),
        Some("application/x-amz-json-1.1")
    );
    assert_eq!(
        header(&outbound, "authorization").as_deref(),
        Some("AWS4-HMAC-SHA256 recorded")
    );
    assert_eq!(
        body_json(&outbound),
        json(r#"{"Document":{"Bytes":"aGVsbG8="}}"#)
    );
}

#[tokio::test]
async fn the_signature_covers_the_final_headers_and_body() {
    let (log, client) = signing_client();
    let request = request("detect-document-text", None, region_params("us-east-1"));

    let outbound = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .expect("the detect request prepares");
    let body = body_bytes(&outbound);

    let calls = log.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(calls[0].url, "https://textract.us-east-1.amazonaws.com/");
    assert_eq!(calls[0].service, "textract");
    assert_eq!(calls[0].region, "us-east-1");
    assert_eq!(calls[0].body, body);
    assert!(
        calls[0].headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("x-amz-target") && value == "Textract.DetectDocumentText"
        }),
        "{:?}",
        calls[0].headers
    );
}

#[tokio::test]
async fn an_analyze_request_defaults_to_layout_and_tables() {
    let (log, client) = signing_client();
    let request = request("analyze-document", None, region_params("ap-south-1"));

    let outbound = TextractAnalyzeDocumentConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .expect("the analyze request prepares");

    assert_eq!(
        outbound.url.as_str(),
        "https://textract.ap-south-1.amazonaws.com/"
    );
    assert_eq!(
        header(&outbound, "x-amz-target").as_deref(),
        Some("Textract.AnalyzeDocument")
    );
    assert_eq!(
        body_json(&outbound),
        json(r#"{"Document":{"Bytes":"aGVsbG8="},"FeatureTypes":["LAYOUT","TABLES"]}"#)
    );
    assert_eq!(log.calls().len(), 1);
    assert_eq!(log.calls()[0].region, "ap-south-1");
}

#[rstest]
#[case::overridden(
    r#"{"feature_types":["FORMS","SIGNATURES"]}"#,
    r#"["FORMS","SIGNATURES"]"#
)]
#[case::explicit_null_uses_the_default(r#"{"feature_types":null}"#, r#"["LAYOUT","TABLES"]"#)]
#[case::explicit_empty_list_is_used_as_given(r#"{"feature_types":[]}"#, "[]")]
#[tokio::test]
async fn feature_types_reach_the_request(#[case] params: &str, #[case] features: &str) {
    let (_log, client) = signing_client();
    let arguments: Object = serde_json::from_str(params).expect("the case params parse");
    let mut request = request("analyze-document", None, arguments);
    with_secret(&mut request, "AWS_REGION_NAME", "us-east-1");

    let outbound = TextractAnalyzeDocumentConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .expect("the analyze request prepares");

    let expected = format!(r#"{{"Document":{{"Bytes":"aGVsbG8="}},"FeatureTypes":{features}}}"#);
    assert_eq!(body_json(&outbound), json(&expected));
}

#[rstest]
#[case::undocumented_feature(r#"{"feature_types":["HANDWRITING"]}"#)]
#[case::lowercase_feature(r#"{"feature_types":["layout"]}"#)]
#[case::not_a_list(r#"{"feature_types":"LAYOUT"}"#)]
#[tokio::test]
async fn feature_types_outside_the_documented_values_are_refused(#[case] params: &str) {
    let (_log, client) = signing_client();
    let arguments: Object = serde_json::from_str(params).expect("the case params parse");
    let request = request("analyze-document", None, arguments);

    let error = TextractAnalyzeDocumentConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .unwrap_err();

    assert!(
        matches!(&error, OcrError::RequestField { path } if path.starts_with("optional_params.feature_types")),
        "{error}"
    );
    assert_eq!(error.http_status_code(), Some(400));
}

#[tokio::test]
async fn the_api_base_replaces_the_regional_endpoint() {
    let (_log, client) = signing_client();
    let request = request(
        "detect-document-text",
        Some("http://127.0.0.1:9/textract"),
        region_params("us-east-1"),
    );

    let outbound = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .expect("the detect request prepares");

    assert_eq!(outbound.url.as_str(), "http://127.0.0.1:9/textract");
}

#[tokio::test]
async fn the_region_option_wins_over_the_environment() {
    let (_log, client) = signing_client();
    let mut request = request("detect-document-text", None, region_params("eu-west-1"));
    with_secret(&mut request, "AWS_REGION_NAME", "ap-south-1");
    with_secret(&mut request, "AWS_REGION", "sa-east-1");

    assert_eq!(regional_url(&request, &client).await, "eu-west-1");
}

#[tokio::test]
async fn the_region_name_environment_variable_comes_before_the_region_one() {
    let (_log, client) = signing_client();
    let mut request = request("detect-document-text", None, Object::new());
    with_secret(&mut request, "AWS_REGION_NAME", "ap-south-1");
    with_secret(&mut request, "AWS_REGION", "sa-east-1");

    assert_eq!(regional_url(&request, &client).await, "ap-south-1");
}

#[tokio::test]
async fn the_region_falls_back_to_the_region_environment_variable() {
    let (_log, client) = signing_client();
    let mut request = request("detect-document-text", None, Object::new());
    with_secret(&mut request, "AWS_REGION", "sa-east-1");

    assert_eq!(regional_url(&request, &client).await, "sa-east-1");
}

#[tokio::test]
async fn a_non_text_region_option_falls_back_to_the_environment() {
    let (_log, client) = signing_client();
    let mut arguments = Object::new();
    let _previous = arguments.insert(
        "aws_region_name".to_owned(),
        Json::Number(serde_json::Number::from(5_i64)),
    );
    let mut request = request("detect-document-text", None, arguments);
    with_secret(&mut request, "AWS_REGION_NAME", "ap-south-1");

    assert_eq!(regional_url(&request, &client).await, "ap-south-1");
}

#[tokio::test]
async fn a_request_without_a_region_is_refused() {
    let (log, client) = signing_client();
    let request = request("detect-document-text", None, Object::new());

    let error = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .unwrap_err();

    assert!(matches!(&error, OcrError::InvalidRequest(_)), "{error}");
    assert_eq!(error.http_status_code(), Some(400));
    assert!(error.to_string().contains("AWS_REGION_NAME"), "{error}");
    assert!(log.calls().is_empty());
}

#[rstest]
#[case::a_region_name("us-east-1", true)]
#[case::a_host_injection("x@evil.example/#", false)]
#[case::an_empty_region("", false)]
#[case::an_uppercase_region("US-EAST-1", false)]
#[tokio::test]
async fn the_default_endpoint_host_needs_a_valid_region(
    #[case] region: &str,
    #[case] allowed: bool,
) {
    let (log, client) = signing_client();
    let request = request("detect-document-text", None, region_params(region));

    let result = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await;

    assert_eq!(result.is_ok(), allowed, "region {region:?}");
    if allowed {
        assert_eq!(
            result.expect("the request prepares").url.as_str(),
            format!("https://textract.{region}.amazonaws.com/")
        );
    } else {
        assert!(
            matches!(&result, Err(OcrError::InvalidRequest(_))),
            "region {region:?}"
        );
        assert_eq!(result.unwrap_err().http_status_code(), Some(400));
        assert!(log.calls().is_empty());
    }
}

#[tokio::test]
async fn the_api_base_does_not_validate_the_region() {
    let (_log, client) = signing_client();
    let request = request(
        "detect-document-text",
        Some("https://example.test/"),
        region_params("x@evil.example/#"),
    );

    let outbound = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .expect("the api_base request prepares");

    assert_eq!(outbound.url.as_str(), "https://example.test/");
}

#[tokio::test]
async fn a_caller_copy_of_an_operation_header_is_replaced() {
    let (log, client) = signing_client();
    let mut request = request(
        "detect-document-text",
        Some("https://example.test/"),
        region_params("us-east-1"),
    );
    request.connection.extra_headers = vec![
        ("content-type".to_owned(), "application/json".to_owned()),
        (
            "X-AMZ-TARGET".to_owned(),
            "Textract.AnalyzeDocument".to_owned(),
        ),
        ("x-trace".to_owned(), "1".to_owned()),
    ];

    let outbound = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .expect("the detect request prepares");

    assert_eq!(outbound.headers.get_all("x-amz-target").iter().count(), 1);
    assert_eq!(
        header(&outbound, "x-amz-target").as_deref(),
        Some("Textract.DetectDocumentText")
    );
    assert_eq!(outbound.headers.get_all("content-type").iter().count(), 1);
    assert_eq!(
        header(&outbound, "content-type").as_deref(),
        Some("application/x-amz-json-1.1")
    );
    assert_eq!(header(&outbound, "x-trace").as_deref(), Some("1"));
    assert_eq!(log.calls().len(), 1);
}

async fn regional_url(request: &PreparedOcrRequest, client: &OcrClient) -> String {
    let outbound = TextractDetectTextConfig
        .prepare_request(request, client, &NoHooks)
        .await
        .expect("the detect request prepares");
    let url = outbound.url.as_str();
    let region = url
        .strip_prefix("https://textract.")
        .and_then(|rest| rest.strip_suffix(".amazonaws.com/"));
    assert!(region.is_some(), "{url}");
    region.unwrap_or_default().to_owned()
}
