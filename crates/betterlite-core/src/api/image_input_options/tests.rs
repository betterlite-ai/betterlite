#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

use rstest::rstest;

use betterlite_tokenizer::image::header::ImageType;

use super::mime::{
    DEFAULT_MIME, SNIFF_BYTES, image_content_type_seekable, image_content_type_stream,
    image_type_from_name, mime_for_image_type,
};
use super::options::{
    Background, ImageEditImage, ImageEditOptionalRequestParams, ImageEditRequestParams,
    InputFidelity, Quality, ResponseFormat,
};
use super::selection::{
    ImageEditProviderConfig, ImageInputOptionsError, OptionalParamsPolicy,
    UNSUPPORTED_PARAMS_STATUS, optional_params_image_edit, requested_optional_params,
};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
const JPEG: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF";
const GIF: &[u8] = b"GIF89a\x01\x00";
const WEBP: &[u8] = b"RIFF\x24\x00\x00\x00WEBPVP8 ";
const HEIC: &[u8] = b"\x00\x00\x00\x18ftypheic";

fn options(source: &str) -> ImageEditOptionalRequestParams {
    serde_json::from_str(source).expect("options")
}

fn empty() -> ImageEditOptionalRequestParams {
    ImageEditOptionalRequestParams::default()
}

#[derive(Debug)]
struct Recorded {
    model: String,
    drop_params: bool,
    options: ImageEditOptionalRequestParams,
}

struct RecordingProvider {
    supported: Vec<String>,
}

impl RecordingProvider {
    fn with(supported: &[&str]) -> Self {
        Self {
            supported: supported.iter().map(|name| (*name).to_owned()).collect(),
        }
    }
}

impl ImageEditProviderConfig for RecordingProvider {
    type Error = ImageInputOptionsError;
    type Mapped = Recorded;

    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.clone()
    }

    fn map_openai_params(
        &self,
        options: &ImageEditOptionalRequestParams,
        model: &str,
        drop_params: bool,
    ) -> Result<Recorded, ImageInputOptionsError> {
        Ok(Recorded {
            model: model.to_owned(),
            drop_params,
            options: options.clone(),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Failure {
    None,
    Tell,
    Seek,
    Read,
    Restore,
}

struct FailingReader {
    cursor: Cursor<Vec<u8>>,
    failure: Failure,
    start_seeks: usize,
    read_sizes: Vec<usize>,
}

fn reader(bytes: &[u8], position: u64, failure: Failure) -> FailingReader {
    let mut cursor = Cursor::new(bytes.to_vec());
    cursor.set_position(position);
    FailingReader {
        cursor,
        failure,
        start_seeks: 0,
        read_sizes: Vec::new(),
    }
}

impl Read for FailingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_sizes.push(buf.len());
        if self.failure == Failure::Read {
            return Err(io::Error::other("read failed"));
        }
        self.cursor.read(buf)
    }
}

impl Seek for FailingReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if self.failure == Failure::Tell && pos == SeekFrom::Current(0) {
            return Err(io::Error::other("tell failed"));
        }
        if matches!(pos, SeekFrom::Start(_)) {
            if self.failure == Failure::Seek && self.start_seeks == 0 {
                self.start_seeks = 1;
                return Err(io::Error::other("seek failed"));
            }
            if self.failure == Failure::Restore && self.start_seeks == 1 {
                self.start_seeks = 2;
                return Err(io::Error::other("restore failed"));
            }
            self.start_seeks = self.start_seeks.saturating_add(1);
        }
        self.cursor.seek(pos)
    }
}

#[test]
fn requested_params_keep_every_declared_value() {
    let source = r#"{"background":"transparent","input_fidelity":"high","mask":"m","n":0,"quality":"low","response_format":"b64_json","size":"1024x1024","user":"","imageConfig":{}}"#;
    let selected = requested_optional_params(&options(source), &[]);
    assert_eq!(selected.background, Some(Background::Transparent));
    assert_eq!(selected.input_fidelity, Some(InputFidelity::High));
    assert_eq!(selected.mask.as_deref(), Some("m"));
    assert_eq!(selected.count, Some(0));
    assert_eq!(selected.quality, Some(Quality::Low));
    assert_eq!(selected.response_format, Some(ResponseFormat::B64Json));
    assert_eq!(selected.size.as_deref(), Some("1024x1024"));
    assert_eq!(selected.user.as_deref(), Some(""));
    assert!(
        selected
            .image_config
            .as_ref()
            .is_some_and(BTreeMap::is_empty)
    );
}

#[rstest]
#[case::zero("0")]
#[case::empty_string("\"\"")]
#[case::not_true("false")]
#[case::empty_object("{}")]
fn requested_params_keep_advertised_extensions(#[case] value: &str) {
    let source = format!(r#"{{"seed":{value}}}"#);
    let supported = ["seed".to_owned()];
    let selected = requested_optional_params(&options(&source), &supported);
    assert_eq!(selected, options(&source));
}

#[test]
fn requested_params_omit_null_and_unadvertised_extensions() {
    let source = r#"{"stream":true,"seed":0,"absent":null}"#;
    let supported = ["seed".to_owned(), "absent".to_owned()];
    let selected = requested_optional_params(&options(source), &supported);
    assert_eq!(selected, options(r#"{"seed":0}"#));
}

#[test]
fn requested_params_omit_declared_nulls() {
    let source = r#"{"mask":null,"n":null,"user":"u"}"#;
    let selected = requested_optional_params(&options(source), &[]);
    assert_eq!(selected, options(r#"{"user":"u"}"#));
}

#[test]
fn unsupported_params_report_the_model_and_names_in_order() {
    let source = r#"{"background":"transparent","input_fidelity":"high","n":0,"response_format":"b64_json","user":"","imageConfig":{},"seed":0}"#;
    let provider = RecordingProvider::with(&["n", "user"]);
    let error = optional_params_image_edit(
        &provider,
        "edit-model",
        &options(source),
        &OptionalParamsPolicy::default(),
    )
    .expect_err("unsupported");
    assert_eq!(
        error,
        ImageInputOptionsError::UnsupportedParams {
            model: "edit-model".to_owned(),
            params: "background, input_fidelity, response_format, imageConfig, seed".to_owned(),
        }
    );
    assert_eq!(UNSUPPORTED_PARAMS_STATUS, 400);
}

#[rstest]
#[case::mask(r#"{"mask":"m"}"#, "mask")]
#[case::quality(r#"{"quality":"low"}"#, "quality")]
#[case::size(r#"{"size":"1024x1024"}"#, "size")]
fn each_unsupported_declared_name_reports_it(#[case] source: &str, #[case] name: &str) {
    let provider = RecordingProvider::with(&["n", "user"]);
    let error = optional_params_image_edit(
        &provider,
        "edit-model",
        &options(source),
        &OptionalParamsPolicy::default(),
    )
    .expect_err("unsupported");
    assert_eq!(
        error,
        ImageInputOptionsError::UnsupportedParams {
            model: "edit-model".to_owned(),
            params: name.to_owned(),
        }
    );
}

#[rstest]
#[case::mask(r#"{"mask":"m"}"#)]
#[case::quality(r#"{"quality":"low"}"#)]
#[case::size(r#"{"size":"1024x1024"}"#)]
fn each_unsupported_declared_name_drops_with_global_flag(#[case] source: &str) {
    let provider = RecordingProvider::with(&["n", "user"]);
    let policy = OptionalParamsPolicy {
        global_drop_params: true,
        call_drop_params: false,
        additional_drop_params: Vec::new(),
    };
    let recorded = optional_params_image_edit(&provider, "edit-model", &options(source), &policy)
        .expect("dropped");
    assert!(recorded.drop_params);
    assert_eq!(recorded.options, empty());
}

#[test]
fn additional_drops_remove_names_before_validation() {
    let source = r#"{"background":"transparent","input_fidelity":"high","n":0,"response_format":"b64_json","user":"","imageConfig":{},"seed":0}"#;
    let provider = RecordingProvider::with(&["n", "user"]);
    let policy = OptionalParamsPolicy {
        global_drop_params: false,
        call_drop_params: false,
        additional_drop_params: [
            "background",
            "input_fidelity",
            "response_format",
            "imageConfig",
            "seed",
            "absent",
        ]
        .map(str::to_owned)
        .to_vec(),
    };
    let recorded = optional_params_image_edit(&provider, "edit-model", &options(source), &policy)
        .expect("dropped");
    assert_eq!(recorded.options, options(r#"{"n":0,"user":""}"#));
}

#[test]
fn additional_drops_remove_the_remaining_declared_names() {
    let provider = RecordingProvider::with(&[]);
    let policy = OptionalParamsPolicy {
        global_drop_params: false,
        call_drop_params: false,
        additional_drop_params: ["mask", "quality", "size", "user"]
            .map(str::to_owned)
            .to_vec(),
    };
    let recorded = optional_params_image_edit(
        &provider,
        "edit-model",
        &options(r#"{"mask":"m","quality":"low","size":"1024x1024","user":"u"}"#),
        &policy,
    )
    .expect("dropped");
    assert_eq!(recorded.options, empty());
}

#[test]
fn additional_drops_remove_supported_names_too() {
    let provider = RecordingProvider::with(&["n", "user"]);
    let policy = OptionalParamsPolicy {
        global_drop_params: false,
        call_drop_params: false,
        additional_drop_params: vec!["n".to_owned()],
    };
    let recorded = optional_params_image_edit(
        &provider,
        "edit-model",
        &options(r#"{"n":0,"user":""}"#),
        &policy,
    )
    .expect("dropped");
    assert_eq!(recorded.options, options(r#"{"user":""}"#));
}

#[rstest]
#[case::global_only(true, false)]
#[case::call_only(false, true)]
#[case::both(true, true)]
fn drop_flag_from_global_or_call(#[case] global: bool, #[case] call: bool) {
    let provider = RecordingProvider::with(&["n", "user"]);
    let policy = OptionalParamsPolicy {
        global_drop_params: global,
        call_drop_params: call,
        additional_drop_params: Vec::new(),
    };
    let recorded =
        optional_params_image_edit(&provider, "edit-model", &options(r#"{"seed":0}"#), &policy)
            .expect("dropped");
    assert!(recorded.drop_params);
    assert_eq!(recorded.options, empty());
}

#[test]
fn neither_drop_flag_reports_unsupported() {
    let provider = RecordingProvider::with(&["n", "user"]);
    let policy = OptionalParamsPolicy {
        global_drop_params: false,
        call_drop_params: false,
        additional_drop_params: Vec::new(),
    };
    let error =
        optional_params_image_edit(&provider, "edit-model", &options(r#"{"seed":0}"#), &policy)
            .expect_err("unsupported");
    assert_eq!(
        error,
        ImageInputOptionsError::UnsupportedParams {
            model: "edit-model".to_owned(),
            params: "seed".to_owned(),
        }
    );
}

#[test]
fn supported_options_are_mapped_with_drop_params_off() {
    let provider = RecordingProvider::with(&["n", "user"]);
    let recorded = optional_params_image_edit(
        &provider,
        "edit-model",
        &options(r#"{"n":0,"user":""}"#),
        &OptionalParamsPolicy::default(),
    )
    .expect("mapped");
    assert_eq!(recorded.model, "edit-model");
    assert!(!recorded.drop_params);
    assert_eq!(recorded.options, options(r#"{"n":0,"user":""}"#));
}

#[rstest]
#[case::png(PNG, "image/png")]
#[case::jpeg(JPEG, "image/jpeg")]
#[case::gif(GIF, "image/gif")]
#[case::webp(WEBP, "image/webp")]
#[case::heic(HEIC, "image/heic")]
fn content_type_from_signature(#[case] bytes: &[u8], #[case] expected: &str) {
    assert_eq!(image_content_type_stream(&mut &bytes[..]), expected);
    assert_eq!(
        image_content_type_seekable(&mut reader(bytes, 0, Failure::None)),
        expected
    );
    assert_eq!(
        image_content_type_stream(&mut reader(bytes, 0, Failure::None)),
        expected
    );
}

#[test]
fn content_type_falls_back_to_png_for_unknown_bytes() {
    assert_eq!(
        image_content_type_stream(&mut &b"not an image"[..]),
        DEFAULT_MIME
    );
    assert_eq!(
        image_content_type_seekable(&mut reader(b"not an image", 0, Failure::None)),
        DEFAULT_MIME
    );
    assert_eq!(
        image_content_type_stream(&mut reader(b"not an image", 0, Failure::None)),
        DEFAULT_MIME
    );
}

#[rstest]
#[case::png(ImageType::Png, "image/png")]
#[case::jpeg(ImageType::Jpeg, "image/jpeg")]
#[case::gif(ImageType::Gif, "image/gif")]
#[case::webp(ImageType::Webp, "image/webp")]
#[case::heic(ImageType::Heic, "image/heic")]
fn image_types_map_to_their_mime(#[case] detected: ImageType, #[case] expected: &str) {
    assert_eq!(mime_for_image_type(Some(detected)), expected);
}

#[test]
fn absent_image_type_falls_back_to_png() {
    assert_eq!(mime_for_image_type(None), DEFAULT_MIME);
}

#[rstest]
#[case::png("png", ImageType::Png)]
#[case::jpeg("jpeg", ImageType::Jpeg)]
#[case::gif("gif", ImageType::Gif)]
#[case::webp("webp", ImageType::Webp)]
#[case::heic("heic", ImageType::Heic)]
fn detected_names_parse_to_image_types(#[case] name: &str, #[case] expected: ImageType) {
    assert_eq!(image_type_from_name(name), Some(expected));
}

#[test]
fn unknown_detected_name_parses_to_none() {
    assert_eq!(image_type_from_name("bmp"), None);
}

#[test]
fn seekable_reads_from_the_start_and_restores_the_position() {
    let mut source = reader(PNG, 7, Failure::None);
    assert_eq!(image_content_type_seekable(&mut source), "image/png");
    assert_eq!(source.cursor.position(), 7);
    assert_eq!(source.read_sizes, vec![SNIFF_BYTES]);
}

#[test]
fn seekable_inspects_no_more_than_the_sniff_limit() {
    let mut bytes = PNG.to_vec();
    bytes.resize(SNIFF_BYTES + 50, 0);
    let mut source = reader(&bytes, 7, Failure::None);
    assert_eq!(image_content_type_seekable(&mut source), "image/png");
    assert_eq!(source.read_sizes, vec![SNIFF_BYTES]);
    assert_eq!(source.cursor.position(), 7);
}

#[test]
fn seekable_tell_failure_returns_png_without_reading() {
    let mut source = reader(PNG, 7, Failure::Tell);
    assert_eq!(image_content_type_seekable(&mut source), DEFAULT_MIME);
    assert_eq!(source.cursor.position(), 7);
    assert!(source.read_sizes.is_empty());
}

#[test]
fn seekable_initial_seek_failure_returns_png_without_reading() {
    let mut source = reader(PNG, 7, Failure::Seek);
    assert_eq!(image_content_type_seekable(&mut source), DEFAULT_MIME);
    assert_eq!(source.cursor.position(), 7);
    assert!(source.read_sizes.is_empty());
}

#[test]
fn seekable_read_failure_returns_png_and_leaves_position_at_zero() {
    let mut source = reader(PNG, 7, Failure::Read);
    assert_eq!(image_content_type_seekable(&mut source), DEFAULT_MIME);
    assert_eq!(source.cursor.position(), 0);
    assert_eq!(source.read_sizes, vec![SNIFF_BYTES]);
}

#[test]
fn seekable_restore_failure_returns_png_after_reading() {
    let mut source = reader(PNG, 7, Failure::Restore);
    assert_eq!(image_content_type_seekable(&mut source), DEFAULT_MIME);
    assert_eq!(
        source.cursor.position(),
        u64::try_from(PNG.len()).expect("length")
    );
    assert_eq!(source.read_sizes, vec![SNIFF_BYTES]);
}

#[test]
fn stream_consumes_the_first_hundred_bytes_without_rewinding() {
    let mut bytes = PNG.to_vec();
    bytes.resize(150, 0);
    let mut source = reader(&bytes, 0, Failure::None);
    assert_eq!(image_content_type_stream(&mut source), "image/png");
    assert_eq!(
        source.cursor.position(),
        u64::try_from(SNIFF_BYTES).expect("limit")
    );
    assert_eq!(source.read_sizes, vec![SNIFF_BYTES]);
}

#[test]
fn stream_read_failure_returns_png() {
    let mut source = reader(PNG, 0, Failure::Read);
    assert_eq!(image_content_type_stream(&mut source), DEFAULT_MIME);
    assert_eq!(source.read_sizes, vec![SNIFF_BYTES]);
}

#[test]
fn request_params_carry_the_image_prompt_and_model() {
    let params = ImageEditRequestParams {
        image: Some(ImageEditImage {
            filename: Some("a.png".to_owned()),
            content_type: Some("image/png".to_owned()),
            content: PNG.to_vec(),
        }),
        prompt: Some("change".to_owned()),
        model: Some("gpt-image-1".to_owned()),
        options: options(r#"{"n":0,"user":""}"#),
    };
    assert_eq!(params.prompt.as_deref(), Some("change"));
    assert_eq!(params.model.as_deref(), Some("gpt-image-1"));
    assert_eq!(
        params.image.as_ref().map(|image| image.content.as_slice()),
        Some(PNG)
    );
    assert_eq!(
        serde_json::to_string(&params.options).expect("json"),
        r#"{"n":0,"user":""}"#
    );
}
