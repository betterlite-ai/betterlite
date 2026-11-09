#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use rstest::rstest;

use crate::gemini_media::equivalence::{equivalent_key, part_exists};
use crate::gemini_media::error::{MediaError, MediaFailure, MediaField};
use crate::gemini_media::mime::{MimeHints, apply_mime_alias};
use crate::gemini_media::part::{MediaResolution, MediaResolutionLevel, VideoMetadata};
use crate::gemini_media::policy::{GoogleProvider, inlines_remote};
use crate::gemini_media::process::{MediaRequest, process_media};
use crate::gemini_media::resolution::{
    convert_detail, highest_media_resolution, max_media_resolution,
};

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

fn expected(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn media_error(url: &str, hints: &MimeHints) -> MediaError {
    process_media(&media_request(url, hints, None, None, None)).unwrap_err()
}

fn video_metadata(source: &str) -> VideoMetadata {
    serde_json::from_str(source).unwrap()
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
fn media_cases_mime_alias(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(apply_mime_alias(input), expected);
}

#[rstest]
#[case::explicit_alias(
    "gs://bucket/photo.bin",
    Some("IMAGE/JPG; charset=utf-8"),
    None,
    None,
    "image/jpeg"
)]
#[case::explicit_bypasses_registry(
    "gs://bucket/photo.bin",
    Some("application/x-custom"),
    None,
    None,
    "application/x-custom"
)]
#[case::format_wins(
    "gs://bucket/a.bin",
    Some("image/png"),
    Some("image/jpeg"),
    Some("image/webp"),
    "image/png"
)]
#[case::mime_type_wins(
    "gs://bucket/a.bin",
    None,
    Some("image/jpeg"),
    Some("image/webp"),
    "image/jpeg"
)]
#[case::content_type_used("gs://bucket/a.bin", None, None, Some("image/webp"), "image/webp")]
#[case::inferred_png("gs://bucket/photo.png", None, None, None, "image/png")]
#[case::inferred_uppercase("gs://bucket/photo.PNG", None, None, None, "image/png")]
#[case::inferred_mp4("gs://bucket/clip.mp4", None, None, None, "video/mp4")]
#[case::inferred_pdf("gs://bucket/doc.pdf", None, None, None, "application/pdf")]
#[case::empty_format_falls_back("gs://bucket/photo.png", Some(""), None, None, "image/png")]
fn media_cases_gcs_hints(
    #[case] url: &str,
    #[case] format: Option<&str>,
    #[case] mime_type: Option<&str>,
    #[case] content_type: Option<&str>,
    #[case] mime: &str,
) {
    let hints = hints(format, mime_type, content_type);
    assert_eq!(
        part_json(url, &hints, None, None, None),
        expected(&format!(
            r#"{{"file_data": {{"mime_type": "{mime}", "file_uri": "{url}"}}}}"#
        ))
    );
}

#[rstest]
#[case::unknown_extension("gs://bucket/photo.xyz", "xyz")]
#[case::query_is_part_of_extension("gs://bucket/photo.png?sig=1", "png?sig=1")]
fn media_cases_gcs_unsupported_extension(#[case] url: &str, #[case] subject: &str) {
    let hints = MimeHints::default();
    assert_eq!(
        media_error(url, &hints),
        MediaError::UnsupportedMediaType {
            subject: subject.to_owned(),
            source: None,
        }
    );
}

#[rstest]
#[case::gif_is_not_accepted("gs://bucket/anim.gif", "image/gif")]
#[case::csv_is_not_accepted("gs://bucket/report.csv", "text/csv")]
fn media_cases_gcs_rejected_type(#[case] url: &str, #[case] subject: &str) {
    let hints = MimeHints::default();
    assert_eq!(
        media_error(url, &hints),
        MediaError::UnsupportedMediaType {
            subject: subject.to_owned(),
            source: None,
        }
    );
}

#[rstest]
#[case::extensionless("gs://bucket/photo")]
#[case::dotfile("gs://bucket/.photo")]
#[case::bare_scheme("gs://")]
#[case::trailing_dot("gs://bucket/photo.")]
fn media_cases_gcs_metadata_lookup_is_unavailable(#[case] url: &str) {
    let hints = MimeHints::default();
    assert_eq!(
        media_error(url, &hints),
        MediaError::RemoteMediaRequired {
            url: url.to_owned(),
            field: MediaField::ImageUrl,
        }
    );
}

#[rstest]
#[case::without_mime("https://generativelanguage.googleapis.com/v1beta/files/abc", None)]
#[case::with_format(
    "https://generativelanguage.googleapis.com/v1beta/files/def",
    Some("image/png")
)]
#[case::raw_format(
    "https://generativelanguage.googleapis.com/v1beta/files/ghi",
    Some("IMAGE/JPG")
)]
fn media_cases_files_api(#[case] url: &str, #[case] format: Option<&str>) {
    let hints = hints(format, None, None);
    let file_data = format.map_or_else(
        || format!(r#"{{"file_uri": "{url}"}}"#),
        |mime| format!(r#"{{"mime_type": "{mime}", "file_uri": "{url}"}}"#),
    );
    assert_eq!(
        part_json(url, &hints, None, None, None),
        expected(&format!(r#"{{"file_data": {file_data}}}"#))
    );
}

#[rstest]
#[case::inferred_jpg("https://example.com/cat.jpg", None, "image/jpeg")]
#[case::inferred_uppercase("https://example.com/cat.JPG", None, "image/jpeg")]
#[case::inferred_ignores_query("https://example.com/cat.png?sig=1", None, "image/png")]
#[case::inferred_mp4("https://example.com/clip.mp4", None, "video/mp4")]
#[case::explicit_format("https://example.com/clip.bin", Some("video/webm"), "video/webm")]
#[case::explicit_format_is_raw("https://example.com/clip.bin", Some("IMAGE/JPG"), "IMAGE/JPG")]
fn media_cases_https_passthrough(
    #[case] url: &str,
    #[case] format: Option<&str>,
    #[case] mime: &str,
) {
    let hints = hints(format, None, None);
    assert_eq!(
        part_json(url, &hints, None, None, None),
        expected(&format!(
            r#"{{"file_data": {{"mime_type": "{mime}", "file_uri": "{url}"}}}}"#
        ))
    );
}

#[rstest]
#[case::http_with_extension("http://example.com/cat.png", None)]
#[case::http_with_format("http://example.com/cat.bin", Some("image/png"))]
#[case::https_without_extension("https://example.com/media", None)]
#[case::https_unknown_extension("https://example.com/media.xyz", None)]
#[case::files_prefix_not_at_start("https://example.com/v1beta/files/abc", None)]
fn media_cases_remote_media_required(#[case] url: &str, #[case] format: Option<&str>) {
    let hints = hints(format, None, None);
    assert_eq!(
        media_error(url, &hints),
        MediaError::RemoteMediaRequired {
            url: url.to_owned(),
            field: MediaField::ImageUrl,
        }
    );
}

#[rstest]
#[case::data_url("data:image/png;base64,YWJj", None, "image/png", "YWJj")]
#[case::format_overrides("data:image/png;base64,YWJj", Some("image/webp"), "image/webp", "YWJj")]
#[case::media_type_parameters(
    "data:image/png;charset=utf-8;base64,YWJj",
    None,
    "image/png;charset=utf-8",
    "YWJj"
)]
fn media_cases_inline_data(
    #[case] url: &str,
    #[case] format: Option<&str>,
    #[case] mime: &str,
    #[case] data: &str,
) {
    let hints = hints(format, None, None);
    assert_eq!(
        part_json(url, &hints, None, None, None),
        expected(&format!(
            r#"{{"inline_data": {{"data": "{data}", "mime_type": "{mime}"}}}}"#
        ))
    );
}

#[rstest]
#[case::alias_not_applied("data:image/jpg;base64,YWJj", "image/jpg")]
#[case::escaped_slash("data:image\\/png;base64,YWJj", "image/png")]
fn media_cases_inline_mime(#[case] url: &str, #[case] mime: &str) {
    let hints = MimeHints::default();
    assert_eq!(
        part_json(url, &hints, None, None, None),
        expected(&format!(
            r#"{{"inline_data": {{"data": "YWJj", "mime_type": "{mime}"}}}}"#
        ))
    );
}

#[rstest]
#[case::no_data_prefix("base64,YWJj")]
#[case::two_markers("data:image/png;base64,a;base64,b")]
#[case::marker_without_comma("data:image/png;base64x,YWJj")]
fn media_cases_invalid_inline_data(#[case] url: &str) {
    let hints = MimeHints::default();
    assert_eq!(
        media_error(url, &hints),
        MediaError::InvalidInlineData {
            value: url.to_owned(),
        }
    );
}

#[rstest]
#[case::plain_text("not a media reference")]
#[case::empty("")]
fn media_cases_invalid_media(#[case] url: &str) {
    let hints = MimeHints::default();
    assert_eq!(
        media_error(url, &hints),
        MediaError::InvalidMedia {
            value: url.to_owned(),
        }
    );
}

#[test]
fn media_cases_failure_wire() {
    let hints = MimeHints::default();
    assert_eq!(
        media_error("gs://bucket/photo", &hints).failure(),
        MediaFailure {
            status: 400,
            error_type: "invalid_request_error",
            param: Some("image_url"),
            code: Some("unsupported_remote_media"),
        }
    );
    assert_eq!(
        media_error("gs://bucket/photo.xyz", &hints).failure(),
        MediaFailure {
            status: 400,
            error_type: "invalid_request_error",
            param: None,
            code: None,
        }
    );
    let request = MediaRequest {
        url: "gs://bucket/photo",
        hints: &hints,
        resolution: None,
        video_metadata: None,
        model: None,
        field: MediaField::File,
    };
    assert_eq!(
        process_media(&request).unwrap_err().failure().param,
        Some("file")
    );
}

#[rstest]
#[case::no_model(
    "gs://bucket/clip.mp4",
    None,
    Some("high"),
    Some(r#"{"fps": 1}"#),
    r#"{"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}}"#
)]
#[case::offsets("gs://bucket/clip.mp4", Some("gemini-2.5-flash"), None, Some(r#"{"fps": 1, "start_offset": "1s", "end_offset": "2.5s"}"#), r#"{"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}, "video_metadata": {"fps": 1, "startOffset": "1s", "endOffset": "2.5s"}}"#)]
#[case::fps_only("gs://bucket/clip.mp4", Some("gemini-2.5-flash"), None, Some(r#"{"fps": 2}"#), r#"{"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}, "video_metadata": {"fps": 2}}"#)]
#[case::empty_metadata(
    "gs://bucket/clip.mp4",
    Some("gemini-2.5-flash"),
    None,
    Some("{}"),
    r#"{"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}}"#
)]
#[case::gemini2_ignores_detail(
    "gs://bucket/clip.mp4",
    Some("gemini-2.5-flash"),
    Some("high"),
    None,
    r#"{"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}}"#
)]
#[case::gemini3_takes_detail("gs://bucket/clip.mp4", Some("gemini-3-pro-preview"), Some("ultra_high"), None, r#"{"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}, "media_resolution": {"level": "MEDIA_RESOLUTION_ULTRA_HIGH"}}"#)]
#[case::unknown_detail_ignored(
    "gs://bucket/clip.mp4",
    Some("gemini-3-pro-preview"),
    Some("bogus"),
    None,
    r#"{"file_data": {"mime_type": "video/mp4", "file_uri": "gs://bucket/clip.mp4"}}"#
)]
#[case::inline_data_keeps_metadata("data:video/mp4;base64,YWJj", Some("gemini-2.5-flash"), None, Some(r#"{"start_offset": "1s"}"#), r#"{"inline_data": {"data": "YWJj", "mime_type": "video/mp4"}, "video_metadata": {"startOffset": "1s"}}"#)]
fn media_cases_metadata(
    #[case] url: &str,
    #[case] model: Option<&str>,
    #[case] detail: Option<&str>,
    #[case] metadata: Option<&str>,
    #[case] part: &str,
) {
    let hints = MimeHints::default();
    let typed_metadata = metadata.map(video_metadata);
    assert_eq!(
        part_json(url, &hints, model, detail, typed_metadata.as_ref()),
        expected(part)
    );
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
#[case::higher_candidate_wins(Some("low"), Some("high"), Some("high"))]
#[case::lower_candidate_loses(Some("ultra_high"), Some("medium"), Some("ultra_high"))]
#[case::none_current(Some("high"), Some("medium"), Some("high"))]
#[case::both_none(None, None, None)]
#[case::unknown_new_keeps_current(Some("low"), Some("bogus"), Some("low"))]
#[case::unknown_current_kept_no_candidate(Some("bogus"), None, Some("bogus"))]
#[case::unknown_current_loses_to_low(Some("bogus"), Some("low"), Some("low"))]
fn media_cases_highest_resolution(
    #[case] current: Option<&str>,
    #[case] candidate: Option<&str>,
    #[case] maximum: Option<&str>,
) {
    assert_eq!(highest_media_resolution(current, candidate), maximum);
}

#[rstest]
#[case::image_and_file_details(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/a.png", "detail": "high"}}, {"type": "file", "file": {"file_data": "gs://bucket/b.mp4", "detail": "medium"}}]}]"#,
    Some("high"),
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

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
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
    let fields = object(available);
    assert_eq!(equivalent_key(key, &fields), found);
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
    let hints = hints(format, None, None);
    assert_eq!(inlines_remote(provider, url, &hints), inlines);
}

fn assert_remote_media_required(hints: &MimeHints, url: &str) {
    let error = process_media(&media_request(url, hints, None, None, None)).unwrap_err();
    assert_eq!(
        error,
        MediaError::RemoteMediaRequired {
            url: url.to_owned(),
            field: MediaField::ImageUrl,
        }
    );
    assert_eq!(
        error.failure(),
        MediaFailure {
            status: 400,
            error_type: "invalid_request_error",
            param: Some("image_url"),
            code: Some("unsupported_remote_media"),
        }
    );
}

#[test]
fn no_remote_fetch() {
    // process_media takes no transport or credential field, so these errors prove no call was made
    let hints = MimeHints::default();
    assert_remote_media_required(&hints, "https://example.com/media");
    assert_remote_media_required(&hints, "gs://bucket/photo");
}
