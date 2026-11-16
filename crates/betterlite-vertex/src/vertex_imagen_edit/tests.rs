#![cfg(test)]

use std::fs;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::PathBuf;

use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_prompts::media::data_url::MAX_MEDIA_BYTES;
use http::header::{HeaderMap, HeaderValue};
use rstest::rstest;

use super::decode::{decode_imagen_edit_input, parse_imagen_edit_input};
use super::error::ImagenEditError;
use super::input::{
    ImagenFields, ImagenFileInput, ImagenFileRead, ImagenReadResult, read_image_bytes,
};
use super::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::request::transform_image_edit_request;
use super::response::{ImagenEditResponse, transform_image_edit_response};
use super::target::predict_url;
use crate::credential_sources::getters::VertexParams;
use crate::vertex_image_identity::error::ImageIdentityError;
use crate::vertex_image_identity::settings::ImageIdentitySettings;
use crate::vertex_image_identity::target::ImageTarget;
use crate::vertex_imagen_generation::error::ImagenGenerationError;

fn params(source: &str) -> ImageParams {
    serde_json::from_str(source).expect("params source")
}

fn json_value(source: &str) -> Json {
    serde_json::from_str(source).expect("json source")
}

fn payload(
    prompt: Option<&str>,
    images: &mut [Option<ImagenFileInput<'_>>],
    mask: Option<&mut ImagenFileInput<'_>>,
    mapped: &ImageParams,
    max_depth: i64,
) -> Json {
    let (text, files) = transform_image_edit_request(prompt, images, mask, mapped, max_depth)
        .expect("the request builds");
    assert!(files.is_empty());
    json_value(&text)
}

fn read_bytes(input: &mut ImagenFileInput<'_>) -> Result<Vec<u8>, ImagenEditError> {
    read_image_bytes(input, 0, 100)
}

fn document(body: &str) -> ImagenFileInput<'static> {
    parse_imagen_edit_input(body.as_bytes())
        .expect("the document parses")
        .expect("the document is an image record")
}

fn read_document(body: &str) -> Result<Vec<u8>, ImagenEditError> {
    read_image_bytes(&mut document(body), 0, 100)
}

// every level adds one to the depth its member is read at
fn nested(depth: usize, leaf: ImagenFileInput<'static>) -> ImagenFileInput<'static> {
    let mut image = leaf;
    for _ in 0..depth {
        image = ImagenFileInput::Sequence(vec![Some(image)]);
    }
    image
}

fn nested_mapping(depth: usize, leaf: ImagenFileInput<'static>) -> ImagenFileInput<'static> {
    let mut image = leaf;
    for _ in 0..depth {
        image = ImagenFileInput::Mapping(ImagenFields {
            data: None,
            bytes: None,
            content: None,
            path: Some(Box::new(image)),
        });
    }
    image
}

fn identity(project: Option<&str>, location: Option<&str>) -> ImageIdentitySettings {
    let mut settings = ImageIdentitySettings::default();
    settings.environment.project = project.map(str::to_owned);
    settings.environment.location = location.map(str::to_owned);
    settings
}

fn reply(status: u16, headers: HeaderMap, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

fn encoded(response: &ImagenEditResponse) -> Json {
    serde_json::from_str(&serde_json::to_string(response).expect("the response encodes"))
        .expect("the response is JSON")
}

struct NoTellStream {
    content: Vec<u8>,
    position: usize,
}

impl Read for NoTellStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = &self.content[self.position..];
        let count = remaining.len().min(buffer.len());
        buffer[..count].copy_from_slice(&remaining[..count]);
        self.position = self.position.saturating_add(count);
        Ok(count)
    }
}

// a tell is a seek to the current position, which this stream refuses
impl Seek for NoTellStream {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match position {
            SeekFrom::Start(offset) => {
                self.position = usize::try_from(offset).expect("a seek offset fits");
                Ok(offset)
            }
            SeekFrom::Current(_) | SeekFrom::End(_) => Err(io::Error::other("tell refused")),
        }
    }
}

struct BrokenStream;

impl Read for BrokenStream {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("read refused"))
    }
}

impl Seek for BrokenStream {
    fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
        Err(io::Error::other("seek refused"))
    }
}

struct BytesReader(Vec<u8>);

impl ImagenFileRead for BytesReader {
    fn read_all(&mut self) -> io::Result<ImagenReadResult> {
        Ok(ImagenReadResult::Bytes(self.0.clone()))
    }
}

struct TextReader(String);

impl ImagenFileRead for TextReader {
    fn read_all(&mut self) -> io::Result<ImagenReadResult> {
        Ok(ImagenReadResult::Text(self.0.clone()))
    }
}

struct BrokenReader;

impl ImagenFileRead for BrokenReader {
    fn read_all(&mut self) -> io::Result<ImagenReadResult> {
        Err(io::Error::other("read refused"))
    }
}

#[test]
fn only_n_size_and_mask_are_advertised() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "size", "mask"]);
}

#[rstest]
#[case::positive(r#"{"n":2}"#, r#"{"sampleCount":2}"#)]
#[case::negative(r#"{"n":-1}"#, r#"{"sampleCount":-1}"#)]
#[case::zero(r#"{"n":0}"#, r#"{"sampleCount":0}"#)]
#[case::absent("{}", "{}")]
#[case::null(r#"{"n":null}"#, "{}")]
fn an_n_maps_to_sample_count(#[case] source: &str, #[case] expected: &str) {
    let mapped = map_openai_params(&params(source)).expect("an integer n maps");
    assert_eq!(Json::Object(mapped), json_value(expected));
}

#[rstest]
#[case::text(r#"{"n":"two"}"#)]
#[case::float(r#"{"n":2.5}"#)]
#[case::flag(r#"{"n":true}"#)]
fn a_non_integer_n_names_the_field(#[case] source: &str) {
    let error = map_openai_params(&params(source)).expect_err("n is an integer");
    assert!(matches!(
        error,
        ImagenEditError::FieldType {
            param: "n",
            expected: "an integer"
        }
    ));
}

#[rstest]
#[case::square("1024x1024", "1:1")]
#[case::wide("1792x1024", "16:9")]
#[case::tall("1024x1792", "9:16")]
#[case::landscape("1280x896", "4:3")]
#[case::portrait("896x1280", "3:4")]
#[case::unknown("999x999", "1:1")]
fn a_size_maps_to_its_aspect_ratio(#[case] size: &str, #[case] ratio: &str) {
    let mapped =
        map_openai_params(&params(&format!(r#"{{"size":"{size}"}}"#))).expect("a string size maps");
    assert_eq!(
        Json::Object(mapped),
        json_value(&format!(r#"{{"aspectRatio":"{ratio}"}}"#))
    );
}

#[test]
fn a_null_size_is_square_and_a_non_string_size_names_the_field() {
    let mapped = map_openai_params(&params(r#"{"size":null}"#)).expect("a null size maps");
    assert_eq!(Json::Object(mapped), json_value(r#"{"aspectRatio":"1:1"}"#));
    let error = map_openai_params(&params(r#"{"size":1792}"#)).expect_err("a number is not a size");
    assert!(matches!(
        error,
        ImagenEditError::FieldType {
            param: "size",
            expected: "a string"
        }
    ));
}

#[test]
fn an_optional_param_outside_the_supported_set_is_dropped() {
    let mapped = map_openai_params(&params(r#"{"quality":"hd","n":2}"#)).expect("n maps");
    assert_eq!(Json::Object(mapped), json_value(r#"{"sampleCount":2}"#));
}

#[test]
fn the_body_carries_the_inpainting_parameters_and_one_raw_reference() {
    let mut images = [Some(ImagenFileInput::Bytes(b"\x89PNG\r\n\x1a\n".to_vec()))];
    let (text, files) =
        transform_image_edit_request(Some("make it blue"), &mut images, None, &params("{}"), 100)
            .expect("the request builds");
    assert!(files.is_empty());
    assert!(text.starts_with(r#"{"instances":[{"prompt":"make it blue","referenceImages":["#));
    assert_eq!(
        json_value(&text),
        json_value(
            r#"{"instances":[{"prompt":"make it blue","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"iVBORw0KGgo="}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn the_mask_follows_the_raw_references_and_repeats_an_id() {
    let mut images = [
        Some(ImagenFileInput::Bytes(b"raw-one".to_vec())),
        None,
        Some(ImagenFileInput::Bytes(b"raw-two".to_vec())),
    ];
    let mut mask = ImagenFileInput::Bytes(b"mask-bytes".to_vec());
    let body = payload(
        Some("fill"),
        &mut images,
        Some(&mut mask),
        &params("{}"),
        100,
    );
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"cmF3LW9uZQ=="}},{"referenceType":"REFERENCE_TYPE_RAW","referenceId":3,"referenceImage":{"bytesBase64Encoded":"cmF3LXR3bw=="}},{"referenceType":"REFERENCE_TYPE_MASK","referenceId":3,"referenceImage":{"bytesBase64Encoded":"bWFzay1ieXRlcw=="},"maskImageConfig":{"maskMode":"MASK_MODE_USER_PROVIDED","dilation":0.03}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn a_mask_alone_follows_an_all_none_image_list() {
    let mut images = [None, None];
    let mut mask = ImagenFileInput::Bytes(b"mask-bytes".to_vec());
    let body = payload(
        Some("fill"),
        &mut images,
        Some(&mut mask),
        &params("{}"),
        100,
    );
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_MASK","referenceId":1,"referenceImage":{"bytesBase64Encoded":"bWFzay1ieXRlcw=="},"maskImageConfig":{"maskMode":"MASK_MODE_USER_PROVIDED","dilation":0.03}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn an_absent_image_is_refused_before_the_mask_is_read() {
    let mut mask = ImagenFileInput::Text("never read".to_owned());
    let error =
        transform_image_edit_request(Some("fill"), &mut [], Some(&mut mask), &params("{}"), 100)
            .expect_err("an absent image is no request");
    assert!(matches!(error, ImagenEditError::MissingImage));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn an_all_none_image_list_without_a_mask_is_refused() {
    let mut images = [None, None];
    let error = transform_image_edit_request(Some("fill"), &mut images, None, &params("{}"), 100)
        .expect_err("no reference image");
    assert!(matches!(error, ImagenEditError::MissingImage));
}

#[test]
fn a_missing_prompt_is_refused_after_the_references_are_read() {
    let mut images = [Some(ImagenFileInput::Bytes(b"raw-one".to_vec()))];
    let error = transform_image_edit_request(None, &mut images, None, &params("{}"), 100)
        .expect_err("a prompt is required");
    assert!(matches!(error, ImagenEditError::MissingPrompt));
    let mut broken = [Some(ImagenFileInput::Text("raw-one".to_owned()))];
    let read_error = transform_image_edit_request(None, &mut broken, None, &params("{}"), 100)
        .expect_err("the image is read first");
    assert!(matches!(read_error, ImagenEditError::PlainString));
}

#[test]
fn an_empty_prompt_is_kept() {
    let mut images = [Some(ImagenFileInput::Bytes(b"\x89PNG\r\n\x1a\n".to_vec()))];
    let body = payload(Some(""), &mut images, None, &params("{}"), 100);
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"iVBORw0KGgo="}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn a_mapped_sample_count_reaches_the_parameters() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let mapped = map_openai_params(&params(r#"{"n":3}"#)).expect("an integer n maps");
    let body = payload(Some("fill"), &mut images, None, &mapped, 100);
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"YWJj"}}]}],"parameters":{"sampleCount":3,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn an_absent_or_null_sample_count_takes_one_sample() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let absent = payload(Some("fill"), &mut images, None, &params("{}"), 100);
    let mut null_images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let null = payload(
        Some("fill"),
        &mut null_images,
        None,
        &params(r#"{"sampleCount":null}"#),
        100,
    );
    assert_eq!(absent, null);
    assert_eq!(
        absent,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"YWJj"}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn a_non_numeric_sample_count_names_the_field() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let error = transform_image_edit_request(
        Some("fill"),
        &mut images,
        None,
        &params(r#"{"sampleCount":"3"}"#),
        100,
    )
    .expect_err("a text sample count is no number");
    assert!(matches!(
        error,
        ImagenEditError::FieldType {
            param: "sampleCount",
            expected: "a number"
        }
    ));
}

#[test]
fn the_mapped_aspect_ratio_never_reaches_the_body() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let (text, _files) = transform_image_edit_request(
        Some("fill"),
        &mut images,
        None,
        &params(r#"{"aspectRatio":"16:9"}"#),
        100,
    )
    .expect("the request builds");
    assert!(!text.contains("aspectRatio"));
}

#[test]
fn the_image_and_the_mask_are_read_at_the_top_level() {
    let mut images = [Some(ImagenFileInput::Bytes(b"abc".to_vec()))];
    let mut mask = ImagenFileInput::Bytes(b"mask-bytes".to_vec());
    let body = payload(Some("fill"), &mut images, Some(&mut mask), &params("{}"), 0);
    assert_eq!(
        body,
        json_value(
            r#"{"instances":[{"prompt":"fill","referenceImages":[{"referenceType":"REFERENCE_TYPE_RAW","referenceId":1,"referenceImage":{"bytesBase64Encoded":"YWJj"}},{"referenceType":"REFERENCE_TYPE_MASK","referenceId":2,"referenceImage":{"bytesBase64Encoded":"bWFzay1ieXRlcw=="},"maskImageConfig":{"maskMode":"MASK_MODE_USER_PROVIDED","dilation":0.03}}]}],"parameters":{"sampleCount":1,"editMode":"EDIT_MODE_INPAINT_INSERTION","editConfig":{"baseSteps":50},"guidanceScale":7.5,"seed":null}}"#
        )
    );
}

#[test]
fn the_shared_depth_limit_is_one_hundred_levels() {
    let limit = Settings::default().max_recurse_depth;
    let levels = usize::try_from(limit).expect("the default limit fits");
    let mut at_limit = nested(levels, ImagenFileInput::Bytes(b"abc".to_vec()));
    assert_eq!(
        read_image_bytes(&mut at_limit, 0, limit).expect("a hundred levels are read"),
        b"abc".as_slice()
    );
    let mut past_limit = nested(
        levels.saturating_add(1),
        ImagenFileInput::Bytes(b"abc".to_vec()),
    );
    let error = read_image_bytes(&mut past_limit, 0, limit).expect_err("a hundred and one are not");
    assert!(matches!(error, ImagenEditError::Depth { max_depth: 100 }));
}

#[test]
fn nested_mappings_count_toward_the_depth_limit() {
    let limit = Settings::default().max_recurse_depth;
    let levels = usize::try_from(limit).expect("the default limit fits");
    let mut at_limit = nested_mapping(levels, ImagenFileInput::Bytes(b"abc".to_vec()));
    assert_eq!(
        read_image_bytes(&mut at_limit, 0, limit).expect("a hundred levels are read"),
        b"abc".as_slice()
    );
    let mut past_limit = nested_mapping(
        levels.saturating_add(1),
        ImagenFileInput::Bytes(b"abc".to_vec()),
    );
    let error = read_image_bytes(&mut past_limit, 0, limit).expect_err("a hundred and one are not");
    assert!(matches!(error, ImagenEditError::Depth { max_depth: 100 }));
}

#[test]
fn a_bytes_or_bytearray_record_is_used_as_it_came() {
    let mut content = ImagenFileInput::Bytes(b"abc".to_vec());
    assert_eq!(
        read_bytes(&mut content).expect("bytes read"),
        b"abc".as_slice()
    );
    let mut array = ImagenFileInput::ByteArray(b"abc".to_vec());
    assert_eq!(
        read_bytes(&mut array).expect("a bytearray reads"),
        b"abc".as_slice()
    );
}

#[test]
fn a_text_or_path_record_is_refused() {
    let mut text = ImagenFileInput::Text("YWJj".to_owned());
    let error = read_bytes(&mut text).expect_err("a bare string is refused");
    assert!(matches!(error, ImagenEditError::PlainString));
    assert!(error.upstream_answer().is_none());
    let mut path = ImagenFileInput::Path(PathBuf::from("image.png"));
    let path_error = read_bytes(&mut path).expect_err("a path is refused");
    assert!(matches!(path_error, ImagenEditError::FilePath));
}

#[test]
fn a_sequence_reads_only_its_first_non_none_member() {
    let mut image = ImagenFileInput::Sequence(vec![
        None,
        Some(ImagenFileInput::Bytes(b"abc".to_vec())),
        Some(ImagenFileInput::Bytes(b"zzz".to_vec())),
    ]);
    assert_eq!(
        read_bytes(&mut image).expect("the first member answers"),
        b"abc".as_slice()
    );
}

#[test]
fn a_sequence_whose_first_member_is_a_string_is_refused() {
    let mut image =
        ImagenFileInput::Sequence(vec![None, Some(ImagenFileInput::Text("YWJj".to_owned()))]);
    let error = read_bytes(&mut image).expect_err("a bare string member is refused");
    assert!(matches!(error, ImagenEditError::PlainString));
}

#[test]
fn a_sequence_without_a_member_is_unsupported() {
    let mut empty = ImagenFileInput::Sequence(Vec::new());
    let error = read_bytes(&mut empty).expect_err("an empty sequence holds no image");
    assert!(matches!(
        error,
        ImagenEditError::UnsupportedImage {
            kind: "a sequence without a member"
        }
    ));
    let mut all_none = ImagenFileInput::Sequence(vec![None, None]);
    let all_none_error = read_bytes(&mut all_none).expect_err("a list of Nones holds no image");
    assert!(matches!(
        all_none_error,
        ImagenEditError::UnsupportedImage {
            kind: "a sequence without a member"
        }
    ));
}

#[test]
fn a_mapping_prefers_data_then_bytes_then_content() {
    let mut all = ImagenFileInput::Mapping(ImagenFields {
        data: Some(Box::new(ImagenFileInput::Bytes(b"data".to_vec()))),
        bytes: Some(Box::new(ImagenFileInput::Bytes(b"bytes".to_vec()))),
        content: Some(Box::new(ImagenFileInput::Bytes(b"content".to_vec()))),
        path: Some(Box::new(ImagenFileInput::Path(PathBuf::from("image.png")))),
    });
    assert_eq!(
        read_bytes(&mut all).expect("data answers"),
        b"data".as_slice()
    );
    let mut without_data = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: Some(Box::new(ImagenFileInput::Bytes(b"bytes".to_vec()))),
        content: Some(Box::new(ImagenFileInput::Bytes(b"content".to_vec()))),
        path: Some(Box::new(ImagenFileInput::Path(PathBuf::from("image.png")))),
    });
    assert_eq!(
        read_bytes(&mut without_data).expect("bytes answer"),
        b"bytes".as_slice()
    );
    let mut content_only = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: None,
        content: Some(Box::new(ImagenFileInput::Bytes(b"content".to_vec()))),
        path: Some(Box::new(ImagenFileInput::Path(PathBuf::from("image.png")))),
    });
    assert_eq!(
        read_bytes(&mut content_only).expect("content answers"),
        b"content".as_slice()
    );
}

#[test]
fn a_mapping_decodes_a_base64_string_in_its_first_key() {
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: Some(Box::new(ImagenFileInput::Text(
            "MDEyMzQ1Njc4OQ==".to_owned(),
        ))),
        bytes: None,
        content: None,
        path: None,
    });
    assert_eq!(
        read_bytes(&mut image).expect("the base64 text decodes"),
        b"0123456789".as_slice()
    );
}

#[test]
fn a_mapping_falls_through_a_base64_string_it_cannot_decode() {
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: Some(Box::new(ImagenFileInput::Text("not base64!".to_owned()))),
        bytes: Some(Box::new(ImagenFileInput::Text("YWJj".to_owned()))),
        content: None,
        path: None,
    });
    assert_eq!(
        read_bytes(&mut image).expect("the next key answers"),
        b"abc".as_slice()
    );
}

#[test]
fn a_mapping_reads_its_path_after_the_value_keys() {
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: None,
        content: None,
        path: Some(Box::new(ImagenFileInput::Sequence(vec![Some(
            ImagenFileInput::Bytes(b"abc".to_vec()),
        )]))),
    });
    assert_eq!(
        read_bytes(&mut image).expect("the path recurses"),
        b"abc".as_slice()
    );
    let mut string_path = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: None,
        content: None,
        path: Some(Box::new(ImagenFileInput::Text("image.png".to_owned()))),
    });
    let error = read_bytes(&mut string_path).expect_err("a string path is refused");
    assert!(matches!(error, ImagenEditError::PlainString));
}

#[test]
fn a_mapping_without_an_image_key_is_unsupported() {
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: None,
        bytes: None,
        content: None,
        path: None,
    });
    let error = read_bytes(&mut image).expect_err("a mapping of nothing holds no image");
    assert!(matches!(
        error,
        ImagenEditError::UnsupportedImage {
            kind: "a mapping without an image key"
        }
    ));
}

#[test]
fn an_unsupported_record_names_its_kind() {
    let mut image = ImagenFileInput::Unsupported { kind: "a number" };
    let error = read_bytes(&mut image).expect_err("a number holds no image");
    assert!(matches!(
        error,
        ImagenEditError::UnsupportedImage { kind: "a number" }
    ));
}

#[test]
fn a_seekable_stream_is_read_whole_and_left_where_it_was() {
    let mut content = Cursor::new(b"0123456789abcdef".to_vec());
    content.seek(SeekFrom::Start(7)).expect("the cursor moves");
    let mut image = ImagenFileInput::Seekable(&mut content);
    assert_eq!(
        read_bytes(&mut image).expect("the stream reads"),
        b"0123456789abcdef".as_slice()
    );
    assert_eq!(content.position(), 7);
}

#[test]
fn a_seekable_stream_whose_tell_refuses_is_a_stream_error() {
    let mut stream = NoTellStream {
        content: b"0123456789".to_vec(),
        position: 7,
    };
    let mut image = ImagenFileInput::Seekable(&mut stream);
    let error = read_bytes(&mut image).expect_err("a BytesIO always answers its tell");
    assert!(matches!(error, ImagenEditError::Stream(_)));
}

#[test]
fn a_seekable_stream_that_cannot_seek_is_a_stream_error() {
    let mut stream = BrokenStream;
    let mut image = ImagenFileInput::Seekable(&mut stream);
    let error = read_bytes(&mut image).expect_err("a stream that cannot seek fails the read");
    assert!(matches!(error, ImagenEditError::Stream(_)));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn a_buffered_stream_with_a_position_is_rewound_and_restored() {
    let mut content = Cursor::new(b"0123456789abcdef".to_vec());
    content.seek(SeekFrom::Start(7)).expect("the cursor moves");
    let mut image = ImagenFileInput::Buffered(&mut content);
    assert_eq!(
        read_bytes(&mut image).expect("the stream reads"),
        b"0123456789abcdef".as_slice()
    );
    assert_eq!(content.position(), 7);
}

#[test]
fn a_buffered_stream_that_cannot_tell_is_read_from_where_it_stands() {
    let mut stream = NoTellStream {
        content: b"0123456789".to_vec(),
        position: 7,
    };
    let mut image = ImagenFileInput::Buffered(&mut stream);
    assert_eq!(
        read_bytes(&mut image).expect("the stream reads"),
        b"789".as_slice()
    );
}

#[test]
fn a_reader_answers_its_bytes_or_its_text() {
    let mut bytes = BytesReader(b"abc".to_vec());
    let mut image = ImagenFileInput::Reader(&mut bytes);
    assert_eq!(
        read_bytes(&mut image).expect("the reader answers"),
        b"abc".as_slice()
    );
    let mut text = TextReader("héllo".to_owned());
    let mut text_image = ImagenFileInput::Reader(&mut text);
    assert_eq!(
        read_bytes(&mut text_image).expect("the text is encoded"),
        "héllo".as_bytes()
    );
}

#[test]
fn a_reader_that_fails_is_a_stream_error() {
    let mut stream = BrokenReader;
    let mut image = ImagenFileInput::Reader(&mut stream);
    let error = read_bytes(&mut image).expect_err("a refused read fails");
    assert!(matches!(error, ImagenEditError::Stream(_)));
}

#[rstest]
#[case::padded("YWJj", b"abc")]
#[case::junk_between_symbols("YW Jj\n", b"abc")]
#[case::padding("YQ==", b"a")]
#[case::missing_padding("MDEy", b"012")]
#[case::empty("", b"")]
fn permissive_base64_decodes(#[case] text: &str, #[case] expected: &[u8]) {
    assert_eq!(
        decode_imagen_edit_input(text).expect("the text decodes"),
        expected
    );
}

#[rstest]
#[case::one_symbol("Y")]
#[case::two_symbols("YW")]
#[case::three_symbols("YWJ")]
#[case::junk_symbols("not base64!")]
#[case::non_ascii("héllo")]
fn an_undecodable_text_is_a_base64_error(#[case] text: &str) {
    let error = decode_imagen_edit_input(text).expect_err("the text does not decode");
    assert!(matches!(error, ImagenEditError::Base64(_)));
}

#[test]
fn an_input_past_the_ceiling_is_refused() {
    let oversized = "A".repeat(MAX_MEDIA_BYTES.saturating_add(1));
    let text_error =
        decode_imagen_edit_input(&oversized).expect_err("the text is past the ceiling");
    assert!(matches!(
        text_error,
        ImagenEditError::TooLarge { size } if size == MAX_MEDIA_BYTES.saturating_add(1)
    ));
    let body_error =
        parse_imagen_edit_input(oversized.as_bytes()).expect_err("the body is past it");
    assert!(matches!(
        body_error,
        ImagenEditError::TooLarge { size } if size == MAX_MEDIA_BYTES.saturating_add(1)
    ));
    let mut image = ImagenFileInput::Mapping(ImagenFields {
        data: Some(Box::new(ImagenFileInput::Text(oversized))),
        bytes: None,
        content: None,
        path: None,
    });
    let field_error = read_bytes(&mut image).expect_err("the data text is past the ceiling");
    assert!(matches!(
        field_error,
        ImagenEditError::TooLarge { size } if size == MAX_MEDIA_BYTES.saturating_add(1)
    ));
}

#[test]
fn an_input_at_the_ceiling_passes_it() {
    let bounded = "A".repeat(MAX_MEDIA_BYTES);
    assert_eq!(
        decode_imagen_edit_input(&bounded)
            .expect("the text decodes")
            .len(),
        MAX_MEDIA_BYTES.saturating_div(4).saturating_mul(3)
    );
    let error = parse_imagen_edit_input(bounded.as_bytes()).expect_err("the body is not JSON");
    assert!(matches!(error, ImagenEditError::InputJson(_)));
}

#[test]
fn a_null_document_is_no_image() {
    assert!(
        parse_imagen_edit_input(b"null")
            .expect("null parses")
            .is_none()
    );
}

#[test]
fn a_document_that_is_not_json_is_an_input_error() {
    let error = parse_imagen_edit_input(b"{\"data\":").expect_err("a truncated document fails");
    assert!(matches!(error, ImagenEditError::InputJson(_)));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn a_document_refuses_what_the_record_refuses() {
    assert!(matches!(
        read_document(r#""plain""#),
        Err(ImagenEditError::PlainString)
    ));
    assert!(matches!(
        read_document("7"),
        Err(ImagenEditError::UnsupportedImage { kind: "a number" })
    ));
    assert!(matches!(
        read_document("true"),
        Err(ImagenEditError::UnsupportedImage { kind: "a boolean" })
    ));
    assert!(matches!(
        read_document(r#"["YWJj", 5]"#),
        Err(ImagenEditError::PlainString)
    ));
    assert!(matches!(
        read_document("[]"),
        Err(ImagenEditError::UnsupportedImage {
            kind: "a sequence without a member"
        })
    ));
    assert!(matches!(
        read_document("{}"),
        Err(ImagenEditError::UnsupportedImage {
            kind: "a mapping without an image key"
        })
    ));
}

#[test]
fn a_document_path_is_refused_like_the_record() {
    assert!(matches!(
        read_document(r#"{"path":"x.png"}"#),
        Err(ImagenEditError::FilePath)
    ));
    assert!(matches!(
        read_document(r#"{"path":["YWJj"]}"#),
        Err(ImagenEditError::PlainString)
    ));
}

#[test]
fn a_document_reads_the_base64_in_its_value_keys() {
    assert_eq!(
        read_document(r#"{"data":"YWJj"}"#).expect("the data decodes"),
        b"abc".as_slice()
    );
    assert_eq!(
        read_document(r#"{"data":"!!","bytes":"YWJj"}"#).expect("the bytes answer"),
        b"abc".as_slice()
    );
    assert_eq!(
        read_document(r#"{"data":{"data":"YWJj"}}"#).expect("the nested mapping answers"),
        b"abc".as_slice()
    );
}

#[test]
fn the_predict_path_hangs_off_the_regional_host() {
    let settings = identity(Some("my-project"), Some("us-central1"));
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: None,
        params: VertexParams::default(),
    };
    assert_eq!(
        predict_url(&settings, &target).expect("the URL builds"),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/imagen-3.0-capability-001:predict"
    );
}

#[test]
fn a_custom_base_replaces_the_host_and_keeps_the_resource_path() {
    let settings = identity(Some("my-project"), Some("europe-west4"));
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: Some("https://proxy.example/"),
        params: VertexParams::default(),
    };
    assert_eq!(
        predict_url(&settings, &target).expect("the URL builds"),
        "https://proxy.example/v1/projects/my-project/locations/europe-west4/publishers/google/models/imagen-3.0-capability-001:predict"
    );
}

#[test]
fn an_empty_base_falls_back_to_the_regional_host() {
    let settings = identity(Some("my-project"), Some("us-central1"));
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: Some(""),
        params: VertexParams::default(),
    };
    assert_eq!(
        predict_url(&settings, &target).expect("the URL builds"),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/imagen-3.0-capability-001:predict"
    );
}

#[test]
fn a_vertex_ai_prefix_is_stripped_from_the_model() {
    let settings = identity(Some("my-project"), Some("us-central1"));
    let target = ImageTarget {
        model: "vertex_ai/imagen-3.0-capability-001",
        api_base: None,
        params: VertexParams::default(),
    };
    assert_eq!(
        predict_url(&settings, &target).expect("the URL builds"),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/imagen-3.0-capability-001:predict"
    );
}

#[test]
fn the_project_and_location_are_required_even_behind_a_custom_base() {
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: Some("https://proxy.example"),
        params: VertexParams::default(),
    };
    let no_project = identity(None, Some("us-central1"));
    assert!(matches!(
        predict_url(&no_project, &target),
        Err(ImagenEditError::Target(ImageIdentityError::MissingTarget))
    ));
    let no_location = identity(Some("my-project"), None);
    assert!(matches!(
        predict_url(&no_location, &target),
        Err(ImagenEditError::Target(ImageIdentityError::MissingTarget))
    ));
}

#[test]
fn a_location_the_host_builder_refuses_is_a_target_error() {
    let settings = identity(Some("my-project"), Some("bad/location"));
    let target = ImageTarget {
        model: "imagen-3.0-capability-001",
        api_base: None,
        params: VertexParams::default(),
    };
    let error = predict_url(&settings, &target).expect_err("the location is refused");
    assert!(matches!(
        error,
        ImagenEditError::Target(ImageIdentityError::Location(_))
    ));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn only_predictions_holding_the_bytes_key_become_images() {
    let body = r#"{"predictions":[{"bytesBase64Encoded":"AAA","prompt":"ignored"},{"raiFilteredReason":"blocked"},{"bytesBase64Encoded":null}]}"#;
    let response = transform_image_edit_response(&reply(200, HeaderMap::new(), body))
        .expect("the reply decodes");
    assert_eq!(
        encoded(&response),
        json_value(r#"{"data":[{"b64_json":"AAA"},{}]}"#)
    );
}

#[test]
fn a_reply_without_predictions_is_an_empty_data_list() {
    let response = transform_image_edit_response(&reply(200, HeaderMap::new(), "{}"))
        .expect("the reply decodes");
    assert_eq!(encoded(&response), json_value(r#"{"data":[]}"#));
}

#[test]
fn a_non_json_reply_keeps_the_upstream_answer() {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    let error = transform_image_edit_response(&reply(502, headers, r#"{"predictions": ["#))
        .expect_err("a truncated reply fails");
    let (status, kept) = error
        .upstream_answer()
        .expect("a parse failure keeps the reply answer");
    assert_eq!(status, 502);
    assert_eq!(
        kept.get("x-request-id"),
        Some(&HeaderValue::from_static("req-1"))
    );
    assert!(matches!(
        error,
        ImagenEditError::Reply(ImagenGenerationError::NotJson { .. })
    ));
}

#[test]
fn a_predictions_value_that_is_not_a_list_is_a_shape_error() {
    let error =
        transform_image_edit_response(&reply(200, HeaderMap::new(), r#"{"predictions":5}"#))
            .expect_err("a number is not a list");
    assert!(matches!(
        error,
        ImagenEditError::Reply(ImagenGenerationError::Shape { .. })
    ));
    assert!(error.upstream_answer().is_none());
}

#[rstest]
fn replays_the_imagen_edit_input_corpus(#[files("fuzz/corpus/imagen_edit_input/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("the corpus file reads");
    let Ok(projected) = parse_imagen_edit_input(&bytes) else {
        return;
    };
    let Some(mut image) = projected else {
        return;
    };
    drop(read_image_bytes(
        &mut image,
        0,
        Settings::default().max_recurse_depth,
    ));
}
