use crate::arguments::json::Json;
use crate::bedrock_processor::error::ProcessorError;

use super::prelude::{Harness, RecordingCachePoints, Settings, list};

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_image_url_string_is_accepted() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":"data:image/png;base64,AAAA"}]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"image":{"format":"png","source":{"bytes":"AAAA"}}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_image_object_passes_its_format() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{
            "url":"data:image/png;base64,AAAA",
            "format":"image/jpeg"
        }}]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"image":{"format":"jpeg","source":{"bytes":"AAAA"}}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_image_without_a_url_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"image_url","image_url":{}}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ImageUrlWithoutAddress { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_user_image_of_another_type_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"image_url","image_url":5}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ImageUrlNotTextOrObject { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_image_url_format_that_is_not_text_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{
            "url":"data:image/png;base64,AAAA","format":5
        }}]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ImageFormatNotText { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_an_unsupported_image_url_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages =
        list(r#"[{"role":"user","content":[{"type":"image_url","image_url":"ftp://x/a.png"}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessorError::Image { block: 0, .. }));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_file_data_block_becomes_a_document() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"file","file":{
            "file_data":"data:application/pdf;base64,JVBERi0=",
            "format":"application/pdf"
        }}]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"document":{
                    "format":"pdf",
                    "source":{"bytes":"JVBERi0="},
                    "name":"DocumentPDFmessages_fdaa7659473d3816_pdf"
                }},
                {"text":"."}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_file_block_without_a_file_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"file"}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::FileObjectMissing { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_file_block_without_media_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"file","file":{}}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::FileWithoutMedia { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_file_block_that_is_not_an_object_is_rejected()
{
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"file","file":5}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessorError::FileNotObject { block: 0 }));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_file_data_that_is_not_a_string_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"file","file":{"file_data":5}}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::FileDataNotText { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_document_block_becomes_a_named_document() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":"application/pdf","data":"JVBERi0xLjQ="
        }}]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"document":{
                    "format":"pdf",
                    "source":{"bytes":"JVBERi0xLjQ="},
                    "name":"Document_310ef6dc639f8350_pdf"
                }},
                {"text":"."}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_non_base64_document_source_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"url","media_type":"application/pdf","data":"x"
        }}]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::DocumentSourceNotBase64 { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_document_without_a_source_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"document"}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::DocumentSourceMissing { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_document_media_type_without_a_subtype_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":"applicationpdf","data":"x"
        }}]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::DocumentMediaWithoutSubtype { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_document_with_an_unsupported_format_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":"application/x-unknown","data":"x"
        }}]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::DocumentFormat { block: 0, .. }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_the_document_placeholder_precedes_a_trailing_cache_point()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":"application/pdf","data":"JVBERi0xLjQ="
        },"cache_control":{"type":"ephemeral"}}]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"document":{
                    "format":"pdf",
                    "source":{"bytes":"JVBERi0xLjQ="},
                    "name":"Document_310ef6dc639f8350_pdf"
                }},
                {"text":"."},
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_duplicate_document_names_are_renamed() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":"application/pdf","data":"JVBERi0xLjQ="
        }}]},
        {"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":"application/pdf","data":"JVBERi0xLjQ="
        }}]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"document":{
                    "format":"pdf",
                    "source":{"bytes":"JVBERi0xLjQ="},
                    "name":"Document_310ef6dc639f8350_pdf"
                }},
                {"document":{
                    "format":"pdf",
                    "source":{"bytes":"JVBERi0xLjQ="},
                    "name":"Document_310ef6dc639f8350_pdf_2"
                }},
                {"text":"."}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_the_document_hash_ignores_whitespace() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":"application/pdf","data":"JVBE Ri0x LjQ="
        }}]}]"#,
    );
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        converted,
        list(
            r#"[{"role":"user","content":[
                {"document":{
                    "format":"pdf",
                    "source":{"bytes":"JVBE Ri0x LjQ="},
                    "name":"Document_310ef6dc639f8350_pdf"
                }},
                {"text":"."}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_file_id_that_is_not_a_string_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"file","file":{"file_id":5}}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessorError::FileIdNotText { block: 0 }));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_file_format_that_is_not_a_string_is_rejected()
{
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"file","file":{
            "file_data":"data:application/pdf;base64,JVBERi0=","format":5
        }}]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::FileFormatNotText { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_document_source_that_is_not_an_object_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(r#"[{"role":"user","content":[{"type":"document","source":5}]}]"#);
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::DocumentSourceNotObject { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_document_without_a_media_type_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","data":"x"
        }}]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::DocumentMediaTypeMissing { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_document_media_type_that_is_not_a_string_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":5,"data":"x"
        }}]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::DocumentMediaTypeNotText { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_document_without_data_is_rejected() {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":"application/pdf"
        }}]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::DocumentDataMissing { block: 0 }
    ));
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_a_document_data_that_is_not_a_string_is_rejected()
 {
    let harness = Harness::new(RecordingCachePoints::accepting());
    let messages = list(
        r#"[{"role":"user","content":[{"type":"document","source":{
            "type":"base64","media_type":"application/pdf","data":5
        }}]}]"#,
    );
    let error = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::DocumentDataNotText { block: 0 }
    ));
}

fn block_at(converted: &[Json], index: usize) -> &Json {
    let Some(Json::Object(message)) = converted.first() else {
        panic!("the merged user message is missing");
    };
    let Some(Json::List(blocks)) = message.get("content") else {
        panic!("the merged user message has no content list");
    };
    &blocks[index]
}

fn document_field(converted: &[Json], index: usize, key: &str) -> Json {
    let Some(document) = block_at(converted, index)
        .as_object()
        .and_then(|fields| fields.get("document"))
        .and_then(Json::as_object)
    else {
        panic!("block {index} is not a document");
    };
    document.get(key).cloned().unwrap_or(Json::Null)
}

fn document_bytes(converted: &[Json], index: usize) -> Json {
    let source = document_field(converted, index, "source");
    source
        .as_object()
        .and_then(|fields| fields.get("bytes"))
        .cloned()
        .unwrap_or_else(|| panic!("document {index} has no bytes"))
}

#[tokio::test]
async fn bedrock_processor_behavior_tests_behavior_the_document_name_hashes_only_the_first_64_kib()
{
    let harness = Harness::new(RecordingCachePoints::accepting());
    let shared = "A".repeat(0x0001_0000);
    let first = format!("{shared}{}", "B".repeat(4464));
    let second = format!("{shared}{}", "C".repeat(4464));
    let messages = list(&format!(
        r#"[{{"role":"user","content":[{{"type":"document","source":{{
            "type":"base64","media_type":"application/pdf","data":"{first}"
        }}}}]}},
        {{"role":"user","content":[{{"type":"document","source":{{
            "type":"base64","media_type":"application/pdf","data":"{second}"
        }}}}]}}]"#
    ));
    let converted = harness
        .convert(&messages, &Settings::default())
        .await
        .unwrap();
    assert_eq!(
        document_field(&converted, 0, "name"),
        Json::Text("Document_43f36e3528732b21_pdf".to_owned())
    );
    assert_eq!(
        document_field(&converted, 1, "name"),
        Json::Text("Document_43f36e3528732b21_pdf_2".to_owned())
    );
    assert_eq!(document_bytes(&converted, 0), Json::Text(first));
    assert_eq!(document_bytes(&converted, 1), Json::Text(second));
    let Some(placeholder) = block_at(&converted, 2).as_object() else {
        panic!("the placeholder is missing");
    };
    assert_eq!(placeholder.get("text"), Some(&Json::Text(".".to_owned())));
}
