use crate::anthropic_invokes::documents::{
    ContentBlockType, FileIdContentType, FileMessageError, content_element_factory,
    infer_file_id_content_type, process_openai_file_message, select_content_block_type,
};
use crate::anthropic_results::images::{ImageChunk, ImageError};

use super::{MapSource, maps, object};

fn chunk(media_type: &str, data: &str) -> ImageChunk {
    ImageChunk {
        kind: "base64".to_owned(),
        media_type: media_type.to_owned(),
        data: data.to_owned(),
    }
}

#[rstest::rstest]
#[case::pdf("application/pdf", ContentBlockType::Document)]
#[case::plain_text("text/plain", ContentBlockType::Document)]
#[case::jpeg("image/jpeg", ContentBlockType::Image)]
#[case::png("image/png", ContentBlockType::Image)]
#[case::gif("image/gif", ContentBlockType::Image)]
#[case::webp("image/webp", ContentBlockType::Image)]
#[case::svg("image/svg+xml", ContentBlockType::ContainerUpload)]
#[case::parameterized_pdf("application/pdf;charset=utf-8", ContentBlockType::ContainerUpload)]
#[case::empty("", ContentBlockType::ContainerUpload)]
fn anthropic_invokes_behavior_tests_behavior_select_content_block_type(
    #[case] format: &str,
    #[case] expected: ContentBlockType,
) {
    assert_eq!(select_content_block_type(format), expected);
}

#[rstest::rstest]
#[case::http("http://host/file.pdf", FileIdContentType::DocumentUrl)]
#[case::https("https://host/file.pdf", FileIdContentType::DocumentUrl)]
#[case::http_prefix_only("httpfoo", FileIdContentType::DocumentUrl)]
#[case::file_id("file-abc", FileIdContentType::ContainerUpload)]
#[case::uppercase("HTTP://host/file.pdf", FileIdContentType::ContainerUpload)]
#[case::empty("", FileIdContentType::ContainerUpload)]
fn anthropic_invokes_behavior_tests_behavior_infer_file_id_content_type(
    #[case] file_id: &str,
    #[case] expected: FileIdContentType,
) {
    assert_eq!(infer_file_id_content_type(file_id), expected);
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_pdf_chunk_becomes_a_document_block() {
    assert_eq!(
        content_element_factory(&chunk("application/pdf", "QUJD")),
        object(
            r#"{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"QUJD"}}"#
        )
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_non_pdf_chunk_becomes_an_image_block() {
    assert_eq!(
        content_element_factory(&chunk("image/png", "QUJD")),
        object(
            r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}"#
        )
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_the_chunk_kind_is_not_copied() {
    let data = chunk("application/pdf", "QUJD");
    let element = content_element_factory(&ImageChunk {
        kind: "url".to_owned(),
        ..data
    });
    assert_eq!(
        element,
        object(
            r#"{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"QUJD"}}"#
        )
    );
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_file_data_becomes_a_base64_document() {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message = object(r#"{"type":"file","file":{"file_data":"data:image/png;base64,QUJD"}}"#);
    assert_eq!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap(),
        object(
            r#"{"type":"document","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}"#
        )
    );
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_file_data_format_overrides_the_media_type() {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message = object(
        r#"{"type":"file","file":{"file_data":"data:image/png;base64,QUJD","format":"application/pdf"}}"#,
    );
    assert_eq!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap(),
        object(
            r#"{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"QUJD"}}"#
        )
    );
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_remote_file_data_is_fetched_then_inlined() {
    let data_urls = maps(&[(
        "https://example.test/file.png",
        "data:image/png;base64,QUJD",
    )]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message = object(
        r#"{"type":"file","file":{"file_data":"https://example.test/file.png","format":"application/pdf"}}"#,
    );
    assert_eq!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap(),
        object(
            r#"{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"QUJD"}}"#
        )
    );
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_a_failed_file_data_fetch_is_an_image_error() {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message = object(r#"{"type":"file","file":{"file_data":"https://example.test/file.png"}}"#);
    assert!(matches!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap_err(),
        FileMessageError::Image(ImageError::Fetch(_))
    ));
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_malformed_file_data_is_an_image_error() {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message = object(r#"{"type":"file","file":{"file_data":"nonsense"}}"#);
    assert!(matches!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap_err(),
        FileMessageError::Image(ImageError::DataUrl)
    ));
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_file_data_wins_over_a_file_id() {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message =
        object(r#"{"type":"file","file":{"file_data":"data:image/png;base64,QUJD","file_id":5}}"#);
    assert_eq!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap(),
        object(
            r#"{"type":"document","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}"#
        )
    );
}

#[rstest::rstest]
#[case::pdf(
    "application/pdf",
    r#"{"type":"document","source":{"type":"file","file_id":"file-abc"}}"#
)]
#[case::plain_text(
    "text/plain",
    r#"{"type":"document","source":{"type":"file","file_id":"file-abc"}}"#
)]
#[case::png(
    "image/png",
    r#"{"type":"image","source":{"type":"file","file_id":"file-abc"}}"#
)]
#[case::webp(
    "image/webp",
    r#"{"type":"image","source":{"type":"file","file_id":"file-abc"}}"#
)]
#[case::other_format("video/mp4", r#"{"type":"container_upload","file_id":"file-abc"}"#)]
#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_a_file_id_uses_the_format_block_type(
    #[case] media_format: &str,
    #[case] expected: &str,
) {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message = object(&format!(
        r#"{{"type":"file","file":{{"file_id":"file-abc","format":"{media_format}"}}}}"#
    ));
    assert_eq!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap(),
        object(expected)
    );
}

#[rstest::rstest]
#[case::url(
    "https://host/file.pdf",
    r#"{"type":"document","source":{"type":"url","url":"https://host/file.pdf"}}"#
)]
#[case::file_id("file-abc", r#"{"type":"container_upload","file_id":"file-abc"}"#)]
#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_a_file_id_without_format_is_inferred(
    #[case] file_id: &str,
    #[case] expected: &str,
) {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message = object(&format!(
        r#"{{"type":"file","file":{{"file_id":"{file_id}"}}}}"#
    ));
    assert_eq!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap(),
        object(expected)
    );
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_an_empty_format_is_used_as_given() {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message =
        object(r#"{"type":"file","file":{"file_id":"https://host/file.pdf","format":""}}"#);
    assert_eq!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap(),
        object(r#"{"type":"container_upload","file_id":"https://host/file.pdf"}"#)
    );
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_an_empty_file_data_is_not_a_fallback() {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message = object(r#"{"type":"file","file":{"file_data":"","file_id":"file-abc"}}"#);
    assert!(matches!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap_err(),
        FileMessageError::Image(ImageError::DataUrl)
    ));
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_a_null_format_is_inferred() {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message =
        object(r#"{"type":"file","file":{"file_id":"https://host/file.pdf","format":null}}"#);
    assert_eq!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap(),
        object(r#"{"type":"document","source":{"type":"url","url":"https://host/file.pdf"}}"#)
    );
}

#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_a_null_file_data_falls_through_to_the_file_id() {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    let message =
        object(r#"{"type":"file","file":{"file_data":null,"file_id":"https://host/file.pdf"}}"#);
    assert_eq!(
        process_openai_file_message(&message, &source)
            .await
            .unwrap(),
        object(r#"{"type":"document","source":{"type":"url","url":"https://host/file.pdf"}}"#)
    );
}

#[rstest::rstest]
#[case::missing(r#"{"type":"file"}"#)]
#[case::null(r#"{"type":"file","file":null}"#)]
#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_a_missing_file_field_is_a_bad_request(
    #[case] raw: &str,
) {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    assert!(matches!(
        process_openai_file_message(&object(raw), &source)
            .await
            .unwrap_err(),
        FileMessageError::MissingFile
    ));
}

#[rstest::rstest]
#[case::file(r#"{"type":"file","file":"x"}"#, "file")]
#[case::file_data(r#"{"type":"file","file":{"file_data":5}}"#, "file_data")]
#[case::file_id(r#"{"type":"file","file":{"file_id":5}}"#, "file_id")]
#[case::format(r#"{"type":"file","file":{"file_id":"f","format":5}}"#, "format")]
#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_a_wrong_typed_file_field_is_named(
    #[case] raw: &str,
    #[case] expected: &'static str,
) {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    assert!(matches!(
        process_openai_file_message(&object(raw), &source)
            .await
            .unwrap_err(),
        FileMessageError::Field { field } if field == expected
    ));
}

#[rstest::rstest]
#[case::neither_key(r#"{"type":"file","file":{"filename":"x"}}"#)]
#[case::null_sources(r#"{"type":"file","file":{"file_data":null,"file_id":null}}"#)]
#[tokio::test]
async fn anthropic_invokes_behavior_tests_behavior_a_file_message_without_a_source_is_refused(
    #[case] raw: &str,
) {
    let data_urls = maps(&[]);
    let source = MapSource {
        data_urls: &data_urls,
    };
    assert!(matches!(
        process_openai_file_message(&object(raw), &source)
            .await
            .unwrap_err(),
        FileMessageError::MissingSource
    ));
}
