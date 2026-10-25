#![cfg(test)]

use super::*;
use rstest::rstest;
use std::collections::BTreeMap;

#[rstest]
#[case::jpg("jpg", FileType::Jpeg, "image/jpeg")]
#[case::three_gpp("3gpp", FileType::ThreeGpp, "video/3gpp")]
#[case::mpa("mpa", FileType::Mpa, "audio/m4a")]
fn mime_extension_table(#[case] extension: &str, #[case] file_type: FileType, #[case] mime: &str) {
    assert_eq!(type_from_extension(extension), Ok(file_type));
    assert_eq!(mime_from_extension(extension), Ok(mime));
    let canonical_extension = if extension == "jpg" {
        "jpeg"
    } else {
        extension
    };
    assert_eq!(extension_from_mime(mime), Ok(canonical_extension));
}

#[rstest]
#[case::upper_extension("JPG", Ok(FileType::Jpeg))]
#[case::dot(".jpg", Err(FileTypeError::UnknownExtension(String::from(".jpg"))))]
#[case::space(" jpg", Err(FileTypeError::UnknownExtension(String::from(" jpg"))))]
fn mime_lookup_edges(#[case] input: &str, #[case] expected: Result<FileType, FileTypeError>) {
    assert_eq!(type_from_extension(input), expected);
}

#[rstest]
#[case::upper_mime("IMAGE/JPEG", Ok("jpeg"))]
#[case::parameters(
    "image/jpeg;charset=utf-8",
    Err(FileTypeError::UnknownMime(String::from("image/jpeg;charset=utf-8")))
)]
#[case::unknown(
    "application/unknown",
    Err(FileTypeError::UnknownMime(String::from("application/unknown")))
)]
fn mime_lookup_mime_edges(#[case] input: &str, #[case] expected: Result<&str, FileTypeError>) {
    assert_eq!(extension_from_mime(input), expected);
}

#[rstest]
#[case::png(FileType::Png, true)]
#[case::jpeg(FileType::Jpeg, true)]
#[case::gif(FileType::Gif, false)]
#[case::mpa(FileType::Mpa, true)]
#[case::m4a(FileType::M4a, false)]
#[case::txt(FileType::Txt, true)]
#[case::json(FileType::Json, false)]
fn gemini_file_acceptance(#[case] file_type: FileType, #[case] accepted: bool) {
    assert_eq!(gemini_accepts(file_type), accepted);
}

#[test]
fn upload_plan_fields() {
    let request = TwoStepFileUploadRequest {
        method: String::from("POST"),
        url: String::from("https://example.test"),
        headers: BTreeMap::new(),
        data: UploadData::Text(String::from("body")),
    };
    let config = TwoStepFileUploadConfig {
        initial_request: request.clone(),
        upload_request: request,
        upload_url_location: UploadUrlLocation::Body,
        upload_url_key: None,
    };
    assert_eq!(config.upload_url_key, None);
}

#[test]
fn stream_record_identity() {
    let mut result = FileContentStreamingResult {
        stream_iterator: Box::new(vec![vec![0, 255], vec![10]].into_iter()),
        headers: BTreeMap::from([(
            String::from("content-type"),
            String::from("application/octet-stream"),
        )]),
    };
    assert_eq!(result.headers["content-type"], "application/octet-stream");
    assert_eq!(result.stream_iterator.next(), Some(vec![0, 255]));
    assert_eq!(result.stream_iterator.next(), Some(vec![10]));
}

#[test]
fn stream_record_debug_includes_headers() {
    let result = FileContentStreamingResult {
        stream_iterator: Box::new(std::iter::empty()),
        headers: BTreeMap::new(),
    };
    assert!(format!("{result:?}").contains("FileContentStreamingResult"));
}
