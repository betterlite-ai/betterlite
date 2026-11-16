use serde_json::{Value, json};

use crate::openai_video_identifiers::ids::encode_character_id;
use crate::openai_video_identifiers::multipart::MediaBody;
use crate::videos::VideoOptions;
use crate::videos::config::{MULTIPART_FORM_DATA, SUPPORTED_PARAMS, map_params};
use crate::videos::create::{CreateVideoForm, VideoCreate, create_request};

use super::support::{API_BASE, MODEL, PROMPT, fixture, options, video_create};

const REFERENCE_URL: &str = "https://example.invalid/a.png";
const PNG_HEAD: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
const JPEG_HEAD: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00";

fn option_fields() -> VideoOptions {
    let character = encode_character_id("char-1", "openai", Some(MODEL));
    let characters = Value::Array(vec![Value::Object(options(&[
        ("id", Value::String(character)),
        ("role", json!("hero")),
    ]))]);
    options(&[
        ("model", json!("option-model")),
        ("prompt", json!("option-prompt")),
        (
            "extra_headers",
            Value::Object(options(&[("x-option", json!("1"))])),
        ),
        ("input_reference", json!("option-reference")),
        ("seconds", json!("4")),
        ("size", json!("720x1280")),
        ("user", json!("user-1")),
        ("characters", characters),
        ("vendor_flag", json!(true)),
    ])
}

fn plain_form(media: Option<MediaBody<'_>>) -> CreateVideoForm<'_> {
    create_request(MODEL, PROMPT, API_BASE, &VideoCreate::default(), media)
}

fn create_form_case() {
    let given = video_create(Value::Object(option_fields()));
    let media = Some(MediaBody::Text(REFERENCE_URL));
    let form = create_request(MODEL, PROMPT, API_BASE, &given, media);
    let mut expected = option_fields();
    for name in ["model", "prompt", "extra_headers", "input_reference"] {
        expected.remove(name);
    }
    expected.insert("model".to_owned(), json!(MODEL));
    expected.insert("prompt".to_owned(), json!(PROMPT));
    let characters = vec![Value::Object(options(&[
        ("id", json!("char-1")),
        ("role", json!("hero")),
    ]))];
    expected.insert("characters".to_owned(), Value::Array(characters));
    assert_eq!(form.fields, expected);
    assert_eq!(form.api_base, API_BASE);
    assert_eq!(form.files.len(), 1);
    let part = form.files.first().unwrap();
    assert_eq!(part.field, "input_reference");
    assert_eq!(part.filename, "input_reference.png");
    assert_eq!(part.content_type, "image/png");
    assert!(matches!(&part.body, MediaBody::Text(text) if *text == REFERENCE_URL));
}

fn create_options_case() {
    assert_eq!(
        SUPPORTED_PARAMS,
        [
            "model",
            "prompt",
            "input_reference",
            "seconds",
            "size",
            "characters",
            "user",
            "extra_headers"
        ]
    );
    assert_eq!(MULTIPART_FORM_DATA.to_string(), "true");
    let given = option_fields();
    let mut mapped = VideoOptions::new();
    map_params(&video_create(Value::Object(given.clone())), &mut mapped);
    assert_eq!(mapped, given);
    let number_seconds = serde_json::from_value::<VideoCreate>(fixture(r#"{"seconds": 4}"#));
    assert!(number_seconds.unwrap_err().is_data());
    let number_character =
        serde_json::from_value::<VideoCreate>(fixture(r#"{"characters": [{"id": 1}]}"#));
    assert!(number_character.unwrap_err().is_data());
}

pub(super) fn create_multipart_case() {
    create_form_case();
    create_options_case();
}

pub(super) fn input_reference_case() {
    let png = plain_form(Some(MediaBody::Bytes(PNG_HEAD)));
    assert_eq!(png.files.len(), 1);
    let png_part = png.files.first().unwrap();
    assert_eq!(png_part.filename, "input_reference.png");
    assert_eq!(png_part.content_type, "image/png");
    assert!(matches!(&png_part.body, MediaBody::Bytes(bytes) if *bytes == PNG_HEAD));

    let jpeg = plain_form(Some(MediaBody::Bytes(JPEG_HEAD)));
    assert_eq!(jpeg.files.first().unwrap().content_type, "image/jpeg");

    let blank = plain_form(Some(MediaBody::Text("")));
    let blank_part = blank.files.first().unwrap();
    assert_eq!(blank_part.content_type, "image/png");
    assert!(matches!(&blank_part.body, MediaBody::Text(text) if text.is_empty()));

    assert!(plain_form(None).files.is_empty());
    let given = video_create(fixture(r#"{"input_reference": "option-reference"}"#));
    let dropped = create_request(MODEL, PROMPT, API_BASE, &given, None);
    assert!(!dropped.fields.contains_key("input_reference"));
    assert!(dropped.files.is_empty());
}
