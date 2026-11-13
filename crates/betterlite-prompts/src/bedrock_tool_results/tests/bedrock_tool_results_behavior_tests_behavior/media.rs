use rstest::rstest;

use crate::arguments::json::Json;
use crate::bedrock_tool_results::error::BedrockToolResultsError;

use super::prelude::{convert_result, expected};

const IMAGE_BLOCK: &str = r#"{"image":{"format":"png","source":{"bytes":"QUJD"}}}"#;
const PDF_NAME: &str = "DocumentPDFmessages_0a702913308f6ec6_pdf";
const DOCUMENT_BLOCK_TEMPLATE: &str =
    r#"{"document":{"format":"pdf","source":{"bytes":"QUJD"},"name":"NAME"}}"#;

async fn convert(part: &str) -> Json {
    let raw_message = format!(r#"{{"role":"tool","tool_call_id":"call_1","content":[{part}]}}"#);
    convert_result(&raw_message, "unused").await.unwrap()
}

fn result_with(blocks: &[&str]) -> Json {
    expected(&format!(
        r#"{{"toolResult":{{"content":[{}],"toolUseId":"call_1"}}}}"#,
        blocks.join(",")
    ))
}

fn document_block() -> String {
    DOCUMENT_BLOCK_TEMPLATE.replace("NAME", PDF_NAME)
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_data_url_string_becomes_an_image_block() {
    assert_eq!(
        convert(r#"{"type":"image_url","image_url":"data:image/png;base64,QUJD"}"#).await,
        result_with(&[IMAGE_BLOCK])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_an_image_url_object_carries_the_url() {
    assert_eq!(
        convert(r#"{"type":"image_url","image_url":{"url":"data:image/png;base64,QUJD"}}"#).await,
        result_with(&[IMAGE_BLOCK])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_an_image_format_override_is_used() {
    assert_eq!(
        convert(
            r#"{"type":"image_url","image_url":{"url":"data:image/png;base64,QUJD","format":"image/webp"}}"#
        )
        .await,
        result_with(&[r#"{"image":{"format":"webp","source":{"bytes":"QUJD"}}}"#])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_document_data_url_becomes_a_document_block()
{
    assert_eq!(
        convert(r#"{"type":"image_url","image_url":"data:application/pdf;base64,QUJD"}"#).await,
        result_with(&[&document_block()])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_video_data_url_is_dropped() {
    assert_eq!(
        convert(r#"{"type":"image_url","image_url":"data:video/mp4;base64,QUJD"}"#).await,
        result_with(&[])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_file_data_url_becomes_a_media_block() {
    assert_eq!(
        convert(r#"{"type":"file","file":{"file_data":"data:image/png;base64,QUJD"}}"#).await,
        result_with(&[IMAGE_BLOCK])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_file_id_is_read_as_the_media_url() {
    assert_eq!(
        convert(
            r#"{"type":"file","file":{"file_id":"data:application/pdf;base64,QUJD","format":"application/pdf"}}"#
        )
        .await,
        result_with(&[&document_block()])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_file_format_override_replaces_the_data_url_type()
 {
    assert_eq!(
        convert(
            r#"{"type":"file","file":{"file_data":"data:image/png;base64,QUJD","format":"image/webp"}}"#
        )
        .await,
        result_with(&[r#"{"image":{"format":"webp","source":{"bytes":"QUJD"}}}"#])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_file_format_override_can_make_a_document() {
    assert_eq!(
        convert(
            r#"{"type":"file","file":{"file_data":"data:image/png;base64,QUJD","format":"application/pdf"}}"#
        )
        .await,
        result_with(&[&document_block()])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_file_id_wins_over_the_file_data() {
    assert_eq!(
        convert(
            r#"{"type":"file","file":{"file_id":"data:image/png;base64,QUJD","file_data":"data:application/pdf;base64,QUJD"}}"#
        )
        .await,
        result_with(&[IMAGE_BLOCK])
    );
}

#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_media_and_text_keep_their_order() {
    assert_eq!(
        convert(
            r#"{"type":"text","text":"shot"},{"type":"image_url","image_url":"data:image/png;base64,QUJD"},{"type":"file","file":{"file_data":"data:application/pdf;base64,QUJD"}}"#
        )
        .await,
        result_with(&[r#"{"text":"shot"}"#, IMAGE_BLOCK, &document_block()])
    );
}

async fn failure(part: &str) -> BedrockToolResultsError {
    let raw_message = format!(r#"{{"role":"tool","tool_call_id":"call_1","content":[{part}]}}"#);
    convert_result(&raw_message, "unused").await.unwrap_err()
}

#[rstest]
#[case::missing_image_url(r#"{"type":"image_url"}"#, "ImageUrlWithoutUrl")]
#[case::null_image_url(r#"{"type":"image_url","image_url":null}"#, "ImageUrlNotTextOrObject")]
#[case::numeric_image_url(r#"{"type":"image_url","image_url":5}"#, "ImageUrlNotTextOrObject")]
#[case::missing_url(r#"{"type":"image_url","image_url":{}}"#, "ImageUrlWithoutAddress")]
#[case::numeric_url(r#"{"type":"image_url","image_url":{"url":5}}"#, "ImageUrlNotText")]
#[case::numeric_format(
    r#"{"type":"image_url","image_url":{"url":"data:image/png;base64,QUJD","format":5}}"#,
    "ImageFormatNotText"
)]
#[case::numeric_file(r#"{"type":"file","file":5}"#, "FileNotObject")]
#[case::string_file(
    r#"{"type":"file","file":"data:image/png;base64,QUJD"}"#,
    "FileNotObject"
)]
#[case::empty_file(r#"{"type":"file","file":{}}"#, "FileWithoutMedia")]
#[case::missing_file(r#"{"type":"file"}"#, "FileWithoutMedia")]
#[case::null_file(r#"{"type":"file","file":null}"#, "FileWithoutMedia")]
#[case::numeric_file_data(r#"{"type":"file","file":{"file_data":5}}"#, "FileDataNotText")]
#[case::numeric_file_id(r#"{"type":"file","file":{"file_id":5}}"#, "FileIdNotText")]
#[case::numeric_file_format(
    r#"{"type":"file","file":{"file_data":"data:image/png;base64,QUJD","format":5}}"#,
    "FileFormatNotText"
)]
#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_an_unusable_media_part_names_its_failure(
    #[case] part: &str,
    #[case] variant: &str,
) {
    let error = failure(part).await;
    assert_eq!(format!("{error:?}").split([' ', '{']).next(), Some(variant));
}

#[rstest]
#[case::unsupported_scheme(r#"{"type":"image_url","image_url":"s3://bucket/image.png"}"#)]
#[case::empty_file_id(r#"{"type":"file","file":{"file_id":""}}"#)]
#[tokio::test]
async fn bedrock_tool_results_behavior_tests_behavior_a_media_failure_keeps_its_cause(
    #[case] part: &str,
) {
    let error = failure(part).await;
    assert!(
        matches!(error, BedrockToolResultsError::Image { .. }),
        "{error:?}"
    );
}
