#![cfg(test)]

use std::collections::BTreeMap;
use std::{fs, path::PathBuf};

use rstest::rstest;

use super::error::MediaMetadataError;
use super::url_mime::{image_mime_type_from_url, infer_content_type};
use super::{ExtractedFileData, FileBody, FileData, extract_file_data, extract_file_metadata};

fn bytes(data: &[u8]) -> FileBody {
    FileBody::Bytes(data.to_vec())
}

enum Scenario {
    Metadata {
        file_data: FileData,
        expected_filename: Option<&'static str>,
        expected_content_type: &'static str,
    },
    DataOk {
        file_data: FileData,
        expected: ExtractedFileData,
    },
    DataErr {
        file_data: FileData,
        expected_error: MediaMetadataError,
    },
    ImageMimeFromUrl {
        url: &'static str,
        expected: Option<&'static str>,
    },
    InferOk {
        url: &'static str,
        content: &'static [u8],
        current_content_type: Option<&'static str>,
        expected: &'static str,
    },
    InferErr {
        url: &'static str,
        content: &'static [u8],
        current_content_type: Option<&'static str>,
        expected_error: MediaMetadataError,
    },
}

fn run(scenario: Scenario) {
    match scenario {
        Scenario::Metadata {
            file_data,
            expected_filename,
            expected_content_type,
        } => {
            let (filename, content_type) = extract_file_metadata(&file_data);
            assert_eq!(filename.as_deref(), expected_filename);
            assert_eq!(content_type, expected_content_type);
        }
        Scenario::DataOk {
            file_data,
            expected,
        } => assert_eq!(extract_file_data(&file_data), Ok(expected)),
        Scenario::DataErr {
            file_data,
            expected_error,
        } => assert_eq!(extract_file_data(&file_data), Err(expected_error)),
        Scenario::ImageMimeFromUrl { url, expected } => {
            assert_eq!(image_mime_type_from_url(url), expected);
        }
        Scenario::InferOk {
            url,
            content,
            current_content_type,
            expected,
        } => assert_eq!(
            infer_content_type(url, content, current_content_type),
            Ok(expected.to_owned())
        ),
        Scenario::InferErr {
            url,
            content,
            current_content_type,
            expected_error,
        } => assert_eq!(
            infer_content_type(url, content, current_content_type),
            Err(expected_error)
        ),
    }
}

#[rstest]
#[case::direct_bytes_without_filename_default_octet_stream(Scenario::Metadata {
    file_data: FileData::Direct(bytes(b"abc")),
    expected_filename: None,
    expected_content_type: "application/octet-stream",
})]
#[case::tuple2_filename_guesses_from_extension(Scenario::Metadata {
    file_data: FileData::Tuple2 { filename: Some("photo.png".to_owned()), content: bytes(b"abc") },
    expected_filename: Some("photo.png"),
    expected_content_type: "image/png",
})]
#[case::tuple3_explicit_content_type_wins_over_guess(Scenario::Metadata {
    file_data: FileData::Tuple3 {
        filename: Some("photo.png".to_owned()),
        content: bytes(b"abc"),
        content_type: Some("application/custom".to_owned()),
    },
    expected_filename: Some("photo.png"),
    expected_content_type: "application/custom",
})]
#[case::tuple3_empty_content_type_falls_back_to_guess(Scenario::Metadata {
    file_data: FileData::Tuple3 {
        filename: Some("photo.png".to_owned()),
        content: bytes(b"abc"),
        content_type: Some(String::new()),
    },
    expected_filename: Some("photo.png"),
    expected_content_type: "image/png",
})]
#[case::leading_dot_filename_has_no_extension(Scenario::Metadata {
    file_data: FileData::Tuple2 { filename: Some(".png".to_owned()), content: bytes(b"abc") },
    expected_filename: Some(".png"),
    expected_content_type: "application/octet-stream",
})]
#[case::nested_path_with_leading_dot_basename_has_no_extension(Scenario::Metadata {
    file_data: FileData::Tuple2 { filename: Some("dir/..gif".to_owned()), content: bytes(b"abc") },
    expected_filename: Some("dir/..gif"),
    expected_content_type: "application/octet-stream",
})]
#[case::dot_after_a_real_character_in_the_basename_still_splits(Scenario::Metadata {
    file_data: FileData::Tuple2 { filename: Some("dir/.a.png".to_owned()), content: bytes(b"abc") },
    expected_filename: Some("dir/.a.png"),
    expected_content_type: "image/png",
})]
#[case::unknown_extension_falls_back_to_octet_stream(Scenario::Metadata {
    file_data: FileData::Tuple2 { filename: Some("archive.zzz".to_owned()), content: bytes(b"abc") },
    expected_filename: Some("archive.zzz"),
    expected_content_type: "application/octet-stream",
})]
#[case::javascript_extension_guesses_text_javascript(Scenario::Metadata {
    file_data: FileData::Tuple2 { filename: Some("app.js".to_owned()), content: bytes(b"x") },
    expected_filename: Some("app.js"),
    expected_content_type: "text/javascript",
})]
#[case::gzip_encoded_pdf_strips_the_encoding_suffix(Scenario::Metadata {
    file_data: FileData::Tuple2 { filename: Some("report.pdf.gz".to_owned()), content: bytes(b"x") },
    expected_filename: Some("report.pdf.gz"),
    expected_content_type: "application/pdf",
})]
#[case::in_memory_file_uses_its_own_content_type(Scenario::Metadata {
    file_data: FileData::InMemoryFile {
        name: "batch.jsonl".to_owned(),
        content_type: "application/jsonl".to_owned(),
        content: b"{}".to_vec(),
    },
    expected_filename: Some("batch.jsonl"),
    expected_content_type: "application/jsonl",
})]
#[case::bare_string_never_reads_the_path_for_metadata(Scenario::Metadata {
    file_data: FileData::Direct(FileBody::BareString("/etc/passwd".to_owned())),
    expected_filename: None,
    expected_content_type: "application/octet-stream",
})]
#[case::extract_file_data_direct_bytes(Scenario::DataOk {
    file_data: FileData::Direct(bytes(b"abc")),
    expected: ExtractedFileData {
        filename: None,
        content: b"abc".to_vec(),
        content_type: "application/octet-stream".to_owned(),
        headers: BTreeMap::new(),
    },
})]
#[case::extract_file_data_tuple4_preserves_headers(Scenario::DataOk {
    file_data: FileData::Tuple4 {
        filename: Some("doc.pdf".to_owned()),
        content: bytes(b"%PDF-1.4"),
        content_type: None,
        headers: BTreeMap::from([("x-trace".to_owned(), "abc123".to_owned())]),
    },
    expected: ExtractedFileData {
        filename: Some("doc.pdf".to_owned()),
        content: b"%PDF-1.4".to_vec(),
        content_type: "application/pdf".to_owned(),
        headers: BTreeMap::from([("x-trace".to_owned(), "abc123".to_owned())]),
    },
})]
#[case::extract_file_data_in_memory_file_has_no_headers(Scenario::DataOk {
    file_data: FileData::InMemoryFile {
        name: "batch.jsonl".to_owned(),
        content_type: "application/jsonl".to_owned(),
        content: b"{\"a\":1}".to_vec(),
    },
    expected: ExtractedFileData {
        filename: Some("batch.jsonl".to_owned()),
        content: b"{\"a\":1}".to_vec(),
        content_type: "application/jsonl".to_owned(),
        headers: BTreeMap::new(),
    },
})]
#[case::extract_file_data_rejects_bare_string(Scenario::DataErr {
    file_data: FileData::Direct(FileBody::BareString("/etc/passwd".to_owned())),
    expected_error: MediaMetadataError::BareStringContent,
})]
#[case::image_mime_type_from_url_jpeg_extension(Scenario::ImageMimeFromUrl {
    url: "https://example.com/image.jpg",
    expected: Some("image/jpeg"),
})]
#[case::image_mime_type_from_url_query_string_is_stripped(Scenario::ImageMimeFromUrl {
    url: "https://example.com/image.png?sig=abc123",
    expected: Some("image/png"),
})]
#[case::image_mime_type_from_url_uppercase_is_lowercased(Scenario::ImageMimeFromUrl {
    url: "HTTPS://EXAMPLE.COM/IMAGE.WEBP",
    expected: Some("image/webp"),
})]
#[case::image_mime_type_from_url_ambiguous_mpeg_prefers_the_earlier_video_entry(Scenario::ImageMimeFromUrl {
    url: "https://example.com/clip.mpeg",
    expected: Some("video/mpeg"),
})]
#[case::image_mime_type_from_url_unknown_extension_returns_none(Scenario::ImageMimeFromUrl {
    url: "https://example.com/file.zzz",
    expected: None,
})]
#[case::image_mime_type_from_url_no_extension_returns_none(Scenario::ImageMimeFromUrl {
    url: "https://example.com/file",
    expected: None,
})]
#[case::image_mime_type_from_url_query_before_any_slash_has_no_path(Scenario::ImageMimeFromUrl {
    url: "https://api.example.com?file=/img.png",
    expected: None,
})]
#[case::image_mime_type_from_url_netloc_only_has_no_path(Scenario::ImageMimeFromUrl {
    url: "https://cdn.example.jpg",
    expected: None,
})]
#[case::image_mime_type_from_url_strips_params_from_the_last_segment(Scenario::ImageMimeFromUrl {
    url: "https://x.com/a.png;v=1",
    expected: Some("image/png"),
})]
#[case::infer_content_type_prefers_declared_type(Scenario::InferOk {
    url: "https://example.com/image.png",
    content: b"ignored",
    current_content_type: Some("image/custom"),
    expected: "image/custom",
})]
#[case::infer_content_type_skips_generic_declared_type(Scenario::InferOk {
    url: "https://example.com/image.png",
    content: b"",
    current_content_type: Some("application/octet-stream"),
    expected: "image/png",
})]
#[case::infer_content_type_skips_binary_octet_stream_declared_type(Scenario::InferOk {
    url: "https://x/a.png",
    content: b"",
    current_content_type: Some("binary/octet-stream"),
    expected: "image/png",
})]
#[case::infer_content_type_skips_empty_declared_type(Scenario::InferOk {
    url: "https://x/a.png",
    content: b"",
    current_content_type: Some(""),
    expected: "image/png",
})]
#[case::infer_content_type_prefers_url_extension_over_sniffed_magic_bytes(Scenario::InferOk {
    url: "https://x/a.pdf",
    content: b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a",
    current_content_type: None,
    expected: "application/pdf",
})]
#[case::infer_content_type_from_url_extension_with_query(Scenario::InferOk {
    url: "https://cdn.example.com/document.pdf?sig=abc",
    content: b"",
    current_content_type: None,
    expected: "application/pdf",
})]
#[case::infer_content_type_sniffs_png_magic_bytes(Scenario::InferOk {
    url: "",
    content: b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00",
    current_content_type: None,
    expected: "image/png",
})]
#[case::infer_content_type_sniffs_gif_magic_bytes(Scenario::InferOk {
    url: "",
    content: b"GIF89a",
    current_content_type: None,
    expected: "image/gif",
})]
#[case::infer_content_type_rejects_gif8_without_a_marker(Scenario::InferErr {
    url: "",
    content: b"GIF8zz",
    current_content_type: None,
    expected_error: MediaMetadataError::ContentTypeUndetermined { url: String::new() },
})]
#[case::infer_content_type_sniffs_jpeg_magic_bytes(Scenario::InferOk {
    url: "",
    content: b"\xff\xd8\xff\xe0",
    current_content_type: None,
    expected: "image/jpeg",
})]
#[case::infer_content_type_sniffs_heic_magic_bytes(Scenario::InferOk {
    url: "",
    content: b"0000ftypheic",
    current_content_type: None,
    expected: "image/heic",
})]
#[case::infer_content_type_sniffs_webp_magic_bytes(Scenario::InferOk {
    url: "",
    content: b"RIFF0000WEBP",
    current_content_type: None,
    expected: "image/webp",
})]
#[case::infer_content_type_rejects_riff_without_webp_marker(Scenario::InferErr {
    url: "",
    content: b"RIFF0000XYZZ",
    current_content_type: None,
    expected_error: MediaMetadataError::ContentTypeUndetermined { url: String::new() },
})]
#[case::infer_content_type_errors_when_undetermined(Scenario::InferErr {
    url: "https://example.com/file",
    content: b"nope",
    current_content_type: None,
    expected_error: MediaMetadataError::ContentTypeUndetermined {
        url: "https://example.com/file".to_owned(),
    },
})]
fn media_metadata_behavior_tests_behavior(#[case] scenario: Scenario) {
    run(scenario);
}

#[rstest]
fn replays_the_media_metadata_corpus(#[files("fuzz/corpus/media_metadata/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let split = bytes.len().min(32);
    let (url_bytes, content) = bytes.split_at(split);
    let url = str::from_utf8(url_bytes).unwrap_or_default();
    drop(infer_content_type(url, content, None));
}
