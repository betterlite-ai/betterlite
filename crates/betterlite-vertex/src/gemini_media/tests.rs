#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use rstest::rstest;
use serde::Deserialize;

use crate::gemini_media::equivalence::{equivalent_key, part_exists};
use crate::gemini_media::error::{MediaError, MediaField};
use crate::gemini_media::mime::{MimeHints, apply_mime_alias};
use crate::gemini_media::part::{MediaResolution, MediaResolutionLevel, VideoMetadata};
use crate::gemini_media::policy::{GoogleProvider, inlines_remote};
use crate::gemini_media::process::{MediaRequest, process_media};
use crate::gemini_media::resolution::{convert_detail, max_media_resolution};

fn hints(format: Option<&str>, mime_type: Option<&str>, content_type: Option<&str>) -> MimeHints {
    MimeHints {
        format: format.map(str::to_owned),
        mime_type: mime_type.map(str::to_owned),
        content_type: content_type.map(str::to_owned),
    }
}

fn media_request<'media>(
    url: &'media str,
    hints: &'media MimeHints,
    model: Option<&'media str>,
    detail: Option<&str>,
    video_metadata: Option<&'media VideoMetadata>,
) -> MediaRequest<'media> {
    MediaRequest {
        url,
        hints,
        resolution: detail.and_then(convert_detail),
        video_metadata,
        model,
        field: MediaField::ImageUrl,
    }
}

fn part_json(
    url: &str,
    hints: &MimeHints,
    model: Option<&str>,
    detail: Option<&str>,
    video_metadata: Option<&VideoMetadata>,
) -> Json {
    let request = media_request(url, hints, model, detail, video_metadata);
    let value = serde_json::to_value(process_media(&request).unwrap()).unwrap();
    serde_json::from_value(value).unwrap()
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn media_error(url: &str, hints: &MimeHints) -> MediaError {
    process_media(&media_request(url, hints, None, None, None)).unwrap_err()
}

fn assert_wire(
    error: &MediaError,
    error_type: Option<&str>,
    param: Option<&str>,
    code: Option<&str>,
) {
    assert_eq!(error.status(), 400);
    assert_eq!(error.error_type(), error_type);
    assert_eq!(error.param(), param);
    assert_eq!(error.code(), code);
}

#[derive(Deserialize)]
struct PartCase {
    url: String,
    #[serde(default)]
    hints: MimeHints,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    video_metadata: Option<VideoMetadata>,
    expected: Json,
}

#[rstest]
#[case::gcs_explicit_alias(r#"{"url": "gs://bucket/photo.bin", "hints": {"format": "IMAGE/JPG; charset=utf-8"}, "expected": {"file_data": {"mime_type": "image/jpeg", "file_uri": "gs://bucket/photo.bin"}}}"#)]
#[case::gcs_explicit_bypasses_registry(r#"{"url": "gs://bucket/photo.bin", "hints": {"format": "application/x-custom"}, "expected": {"file_data": {"mime_type": "application/x-custom", "file_uri": "gs://bucket/photo.bin"}}}"#)]
#[case::gcs_format_wins(r#"{"url": "gs://bucket/a.bin", "hints": {"format": "image/png", "mime_type": "image/jpeg", "content_type": "image/webp"}, "expected": {"file_data": {"mime_type": "image/png", "file_uri": "gs://bucket/a.bin"}}}"#)]
#[case::gcs_mime_type_wins(r#"{"url": "gs://bucket/a.bin", "hints": {"mime_type": "image/jpeg", "content_type": "image/webp"}, "expected": {"file_data": {"mime_type": "image/jpeg", "file_uri": "gs://bucket/a.bin"}}}"#)]
#[case::gcs_content_type_used(r#"{"url": "gs://bucket/a.bin", "hints": {"content_type": "image/webp"}, "expected": {"file_data": {"mime_type": "image/webp", "file_uri": "gs://bucket/a.bin"}}}"#)]
#[case::gcs_inferred_png(r#"{"url": "gs://bucket/photo.png", "expected": {"file_data": {"mime_type": "image/png", "file_uri": "gs://bucket/photo.png"}}}"#)]
#[case::gcs_inferred_uppercase(r#"{"url": "gs://bucket/photo.PNG", "expected": {"file_data": {"mime_type": "image/png", "file_uri": "gs://bucket/photo.PNG"}}}"#)]
#[case::gcs_inferred_mp4(r#"{"url": "gs://bucket/clip.mp4", "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}}}"#)]
#[case::gcs_inferred_pdf(r#"{"url": "gs://bucket/doc.pdf", "expected": {"file_data": {"mime_type": "application/pdf", "file_uri": "gs://bucket/doc.pdf"}}}"#)]
#[case::gcs_empty_format_falls_back(r#"{"url": "gs://bucket/photo.png", "hints": {"format": ""}, "expected": {"file_data": {"mime_type": "image/png", "file_uri": "gs://bucket/photo.png"}}}"#)]
#[case::files_without_mime(r#"{"url": "https://generativelanguage.googleapis.com/v1beta/files/abc", "expected": {"file_data": {"file_uri": "https://generativelanguage.googleapis.com/v1beta/files/abc"}}}"#)]
#[case::files_with_format(r#"{"url": "https://generativelanguage.googleapis.com/v1beta/files/def", "hints": {"format": "image/png"}, "expected": {"file_data": {"mime_type": "image/png", "file_uri": "https://generativelanguage.googleapis.com/v1beta/files/def"}}}"#)]
#[case::files_raw_format(r#"{"url": "https://generativelanguage.googleapis.com/v1beta/files/ghi", "hints": {"format": "IMAGE/JPG"}, "expected": {"file_data": {"mime_type": "IMAGE/JPG", "file_uri": "https://generativelanguage.googleapis.com/v1beta/files/ghi"}}}"#)]
#[case::https_inferred_jpg(r#"{"url": "https://example.com/cat.jpg", "expected": {"file_data": {"mime_type": "image/jpeg", "file_uri": "https://example.com/cat.jpg"}}}"#)]
#[case::https_inferred_uppercase(r#"{"url": "https://example.com/cat.JPG", "expected": {"file_data": {"mime_type": "image/jpeg", "file_uri": "https://example.com/cat.JPG"}}}"#)]
#[case::https_ignores_query(r#"{"url": "https://example.com/cat.png?sig=1", "expected": {"file_data": {"mime_type": "image/png", "file_uri": "https://example.com/cat.png?sig=1"}}}"#)]
#[case::https_inferred_mp4(r#"{"url": "https://example.com/clip.mp4", "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "https://example.com/clip.mp4"}}}"#)]
#[case::https_explicit_format(r#"{"url": "https://example.com/clip.bin", "hints": {"format": "video/webm"}, "expected": {"file_data": {"mime_type": "video/webm", "file_uri": "https://example.com/clip.bin"}}}"#)]
#[case::https_format_is_raw(r#"{"url": "https://example.com/clip.bin", "hints": {"format": "IMAGE/JPG"}, "expected": {"file_data": {"mime_type": "IMAGE/JPG", "file_uri": "https://example.com/clip.bin"}}}"#)]
#[case::inline_data(r#"{"url": "data:image/png;base64,YWJj", "expected": {"inline_data": {"data": "YWJj", "mime_type": "image/png"}}}"#)]
#[case::inline_format_overrides(r#"{"url": "data:image/png;base64,YWJj", "hints": {"format": "image/webp"}, "expected": {"inline_data": {"data": "YWJj", "mime_type": "image/webp"}}}"#)]
#[case::inline_media_type_parameters(r#"{"url": "data:image/png;charset=utf-8;base64,YWJj", "expected": {"inline_data": {"data": "YWJj", "mime_type": "image/png;charset=utf-8"}}}"#)]
#[case::inline_alias_not_applied(r#"{"url": "data:image/jpg;base64,YWJj", "expected": {"inline_data": {"data": "YWJj", "mime_type": "image/jpg"}}}"#)]
#[case::inline_escaped_slash(r#"{"url": "data:image\\/png;base64,YWJj", "expected": {"inline_data": {"data": "YWJj", "mime_type": "image/png"}}}"#)]
#[case::metadata_no_model(r#"{"url": "gs://bucket/clip.mp4", "detail": "high", "video_metadata": {"fps": 1}, "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}}}"#)]
#[case::metadata_offsets(r#"{"url": "gs://bucket/clip.mp4", "model": "gemini-2.5-flash", "video_metadata": {"fps": 1, "start_offset": "1s", "end_offset": "2.5s"}, "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}, "video_metadata": {"fps": 1, "startOffset": "1s", "endOffset": "2.5s"}}}"#)]
#[case::metadata_fps_only(r#"{"url": "gs://bucket/clip.mp4", "model": "gemini-2.5-flash", "video_metadata": {"fps": 2}, "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}, "video_metadata": {"fps": 2}}}"#)]
#[case::metadata_empty(r#"{"url": "gs://bucket/clip.mp4", "model": "gemini-2.5-flash", "video_metadata": {}, "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}}}"#)]
#[case::metadata_null_value(r#"{"url": "gs://bucket/clip.mp4", "model": "gemini-2.5-flash", "video_metadata": {"fps": null}, "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}, "video_metadata": {"fps": null}}}"#)]
#[case::metadata_gemini2_ignores_detail(r#"{"url": "gs://bucket/clip.mp4", "model": "gemini-2.5-flash", "detail": "high", "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}}}"#)]
#[case::metadata_gemini3_takes_detail(r#"{"url": "gs://bucket/clip.mp4", "model": "gemini-3-pro-preview", "detail": "ultra_high", "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}, "media_resolution": {"level": "MEDIA_RESOLUTION_ULTRA_HIGH"}}}"#)]
#[case::metadata_unknown_detail_ignored(r#"{"url": "gs://bucket/clip.mp4", "model": "gemini-3-pro-preview", "detail": "bogus", "expected": {"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}}}"#)]
#[case::metadata_inline_data(r#"{"url": "data:video/mp4;base64,YWJj", "model": "gemini-2.5-flash", "video_metadata": {"start_offset": "1s"}, "expected": {"inline_data": {"data": "YWJj", "mime_type": "video/mp4"}, "video_metadata": {"startOffset": "1s"}}}"#)]
fn media_cases_parts(#[case] source: &str) {
    let case: PartCase = serde_json::from_str(source).unwrap();
    let part = part_json(
        &case.url,
        &case.hints,
        case.model.as_deref(),
        case.detail.as_deref(),
        case.video_metadata.as_ref(),
    );
    assert_eq!(part, case.expected);
}

#[rstest]
#[case::jpg("image/jpg", "image/jpeg")]
#[case::uppercase("IMAGE/JPG", "image/jpeg")]
#[case::parameters("IMAGE/JPG; charset=utf-8", "image/jpeg")]
#[case::surrounding_space(" image/jpg ; q=1", "image/jpeg")]
#[case::canonical("image/jpeg", "image/jpeg")]
#[case::other_type("application/pdf; charset=utf-8", "application/pdf")]
#[case::unknown("application/x-custom", "application/x-custom")]
#[case::empty("", "")]
fn media_cases_mime_alias(#[case] input: &str, #[case] alias: &str) {
    assert_eq!(apply_mime_alias(input), alias);
}

#[rstest]
#[case::unknown_extension("gs://bucket/photo.xyz", "xyz")]
#[case::query_is_part_of_extension("gs://bucket/photo.png?sig=1", "png?sig=1")]
#[case::gif_is_not_accepted("gs://bucket/anim.gif", "image/gif")]
#[case::csv_is_not_accepted("gs://bucket/report.csv", "text/csv")]
fn media_cases_rejected_media(#[case] url: &str, #[case] subject: &str) {
    let error = media_error(url, &MimeHints::default());
    assert_eq!(
        error,
        MediaError::UnsupportedMediaType(subject.to_owned(), None)
    );
    assert_wire(&error, None, None, Some("400"));
}

#[rstest]
#[case::extensionless("gs://bucket/photo", None)]
#[case::dotfile("gs://bucket/.photo", None)]
#[case::dotdot_prefix("gs://bucket/..png", None)]
#[case::triple_dot_prefix("gs://bucket/...jpg", None)]
#[case::bare_scheme("gs://", None)]
#[case::trailing_dot("gs://bucket/photo.", None)]
#[case::http_with_extension("http://example.com/cat.png", None)]
#[case::http_with_format("http://example.com/cat.bin", Some("image/png"))]
#[case::https_without_extension("https://example.com/media", None)]
#[case::https_unknown_extension("https://example.com/media.xyz", None)]
#[case::files_prefix_not_at_start("https://example.com/v1beta/files/abc", None)]
fn media_cases_remote_media_required(#[case] url: &str, #[case] format: Option<&str>) {
    let error = media_error(url, &hints(format, None, None));
    assert_eq!(error, MediaError::RemoteMediaRequired(MediaField::ImageUrl));
    assert_wire(
        &error,
        Some("invalid_request_error"),
        Some("image_url"),
        Some("unsupported_remote_media"),
    );
}

#[rstest]
#[case::no_data_prefix("base64,YWJj", true)]
#[case::two_markers("data:image/png;base64,a;base64,b", true)]
#[case::marker_without_comma("data:image/png;base64x,YWJj", true)]
#[case::plain_text("not a media reference", false)]
#[case::empty("", false)]
fn media_cases_invalid_reference(#[case] url: &str, #[case] inline: bool) {
    let hints = MimeHints::default();
    let error = if inline {
        MediaError::InvalidInlineData(url.to_owned())
    } else {
        MediaError::InvalidMedia(url.to_owned())
    };
    assert_eq!(media_error(url, &hints), error);
    assert_wire(&error, Some("invalid_request_error"), None, None);
}

#[rstest]
#[case::low("low", Some(MediaResolution { level: MediaResolutionLevel::Low }))]
#[case::ultra_high("ultra_high", Some(MediaResolution { level: MediaResolutionLevel::UltraHigh }))]
#[case::unknown("auto", None)]
#[case::case_sensitive("HIGH", None)]
fn media_cases_convert_detail(#[case] detail: &str, #[case] resolution: Option<MediaResolution>) {
    assert_eq!(convert_detail(detail), resolution);
}

#[rstest]
#[case::image_and_file_details(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/a.png", "detail": "high"}}, {"type": "file", "file": {"file_data": "gs://bucket/b.mp4", "detail": "medium"}}]}]"#,
    Some("high"),
)]
#[case::later_lower_detail_keeps_high(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/a.png", "detail": "high"}}]}, {"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/b.png", "detail": "low"}}]}]"#,
    Some("high"),
)]
#[case::ultra_high_wins(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/a.png", "detail": "high"}}]}, {"role": "user", "content": [{"type": "file", "file": {"file_data": "gs://bucket/b.mp4", "detail": "ultra_high"}}]}]"#,
    Some("ultra_high"),
)]
#[case::string_content_ignored(r#"[{"role": "user", "content": "hello"}]"#, None)]
#[case::empty_detail_ignored(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/a.png", "detail": ""}}]}]"#,
    None,
)]
#[case::unknown_detail_ignored(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/a.png", "detail": "bogus"}}]}]"#,
    None,
)]
#[case::non_media_part_ignored(
    r#"[{"role": "user", "content": [{"type": "text", "text": "hi", "detail": "high"}]}]"#,
    None
)]
#[case::string_image_url_ignored(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": "https://example.com/a.png", "detail": "high"}]}]"#,
    None,
)]
fn media_cases_max_resolution(#[case] source: &str, #[case] maximum: Option<&str>) {
    let messages: Vec<Message> = serde_json::from_str(source).unwrap();
    assert_eq!(max_media_resolution(&messages), maximum);
}

#[rstest]
#[case::exact("text", r#"{"text": "hi"}"#, Some("text"))]
#[case::camel_to_snake(
    "function_call",
    r#"{"functionCall": {"name": "f"}}"#,
    Some("functionCall")
)]
#[case::snake_to_camel(
    "functionCall",
    r#"{"function_call": {"name": "f"}}"#,
    Some("function_call")
)]
#[case::missing("other", r#"{"text": "hi"}"#, None)]
fn media_cases_equivalent_key(
    #[case] key: &str,
    #[case] available: &str,
    #[case] found: Option<&str>,
) {
    assert_eq!(equivalent_key(key, &object(available)), found);
}

#[rstest]
#[case::camel_matches(r#"[{"functionCall": {"name": "f"}}]"#, r#"{"function_call": {"name": "f"}}"#, &[], true)]
#[case::value_differs(r#"[{"function_call": {"name": "f"}}]"#, r#"{"function_call": {"name": "g"}}"#, &[], false)]
#[case::extra_candidate_keys(r#"[{"function_call": {"name": "f"}, "thoughtSignature": "sig"}]"#, r#"{"function_call": {"name": "f"}}"#, &[], true)]
#[case::excluded_key_differs(r#"[{"function_call": {"name": "f"}, "thoughtSignature": "sig"}]"#, r#"{"function_call": {"name": "f"}, "thoughtSignature": "other"}"#, &["thoughtSignature"], true)]
#[case::excluded_only_part(r#"[{"text": "hi"}]"#, r#"{"thoughtSignature": "sig"}"#, &["thoughtSignature"], true)]
#[case::no_candidates("[]", r#"{"text": "hi"}"#, &[], false)]
#[case::nested_values_differ(r#"[{"function_call": {"name": "f", "args": {"a": 1}}}]"#, r#"{"function_call": {"name": "f", "args": {"a": 2}}}"#, &[], false)]
fn media_cases_part_equivalence(
    #[case] parts: &str,
    #[case] part: &str,
    #[case] excluded: &[&str],
    #[case] found: bool,
) {
    let candidates: Vec<Object> = serde_json::from_str(parts).unwrap();
    assert_eq!(part_exists(&candidates, &object(part), excluded), found);
}

#[rstest]
#[case::ai_studio_remote(GoogleProvider::AiStudio, "https://example.com/cat.png", None, true)]
#[case::ai_studio_http(GoogleProvider::AiStudio, "http://example.com/cat.png", None, true)]
#[case::ai_studio_files(
    GoogleProvider::AiStudio,
    "https://generativelanguage.googleapis.com/v1beta/files/abc",
    None,
    false
)]
#[case::ai_studio_gs(GoogleProvider::AiStudio, "gs://bucket/cat.png", None, false)]
#[case::vertex_http(GoogleProvider::Vertex, "http://example.com/cat.png", None, true)]
#[case::vertex_https_inferable(GoogleProvider::Vertex, "https://example.com/cat.png", None, false)]
#[case::vertex_https_with_hint(
    GoogleProvider::Vertex,
    "https://example.com/media",
    Some("image/png"),
    false
)]
#[case::vertex_https_unknown(GoogleProvider::Vertex, "https://example.com/media", None, true)]
#[case::vertex_https_unknown_extension(
    GoogleProvider::Vertex,
    "https://example.com/media.xyz",
    None,
    true
)]
#[case::vertex_files(
    GoogleProvider::Vertex,
    "https://generativelanguage.googleapis.com/v1beta/files/abc",
    None,
    false
)]
#[case::vertex_gs(GoogleProvider::Vertex, "gs://bucket/media", None, false)]
fn media_cases_inline_policy(
    #[case] provider: GoogleProvider,
    #[case] url: &str,
    #[case] format: Option<&str>,
    #[case] inlines: bool,
) {
    assert_eq!(
        inlines_remote(provider, url, &hints(format, None, None)),
        inlines
    );
}

#[test]
fn media_cases_field_names() {
    let hints = MimeHints::default();
    let request = MediaRequest {
        url: "gs://bucket/photo",
        hints: &hints,
        resolution: None,
        video_metadata: None,
        model: None,
        field: MediaField::File,
    };
    let error = process_media(&request).unwrap_err();
    assert_eq!(error, MediaError::RemoteMediaRequired(MediaField::File));
    assert_eq!(error.param(), Some("file"));
}

fn assert_remote_media_required(hints: &MimeHints, url: &str) {
    let error = process_media(&media_request(url, hints, None, None, None)).unwrap_err();
    assert_eq!(error, MediaError::RemoteMediaRequired(MediaField::ImageUrl));
}

#[test]
fn no_remote_fetch() {
    // process_media takes no transport or credential field, so these errors prove no call was made
    let hints = MimeHints::default();
    assert_remote_media_required(&hints, "https://example.com/media");
    assert_remote_media_required(&hints, "gs://bucket/photo");
}
