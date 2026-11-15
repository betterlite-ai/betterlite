use serde_json::{Value, json};

use crate::openai_video_identifiers::ids::encode_character_id;
use crate::openai_video_identifiers::multipart::MediaBody;
use crate::videos::VideoOptions;
use crate::videos::config::{MULTIPART_FORM_DATA, SUPPORTED_PARAMS, map_params};
use crate::videos::create::{CreateVideoForm, VideoCreate, create_request};

use super::support::{API_BASE, MODEL, PROMPT, video_create};

const REFERENCE_URL: &str = "https://example.invalid/a.png";
const PNG_HEAD: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
const JPEG_HEAD: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00";

fn option_fixture() -> Value {
    let character = encode_character_id("char-1", "openai", Some(MODEL));
    json!({
        "model": "option-model",
        "prompt": "option-prompt",
        "extra_headers": {"x-option": "1"},
        "input_reference": "option-reference",
        "seconds": "4",
        "size": "720x1280",
        "user": "user-1",
        "characters": [{"id": character, "role": "hero"}],
        "vendor_flag": true,
    })
}

fn plain_form(media: Option<MediaBody<'_>>) -> CreateVideoForm<'_> {
    create_request(MODEL, PROMPT, API_BASE, &VideoCreate::default(), media)
}

fn create_form_case() {
    let given = video_create(option_fixture());
    let media = Some(MediaBody::Text(REFERENCE_URL));
    let form = create_request(MODEL, PROMPT, API_BASE, &given, media);
    assert_eq!(
        Value::Object(form.fields),
        json!({
            "model": MODEL,
            "prompt": PROMPT,
            "seconds": "4",
            "size": "720x1280",
            "user": "user-1",
            "characters": [{"id": "char-1", "role": "hero"}],
            "vendor_flag": true,
        })
    );
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
    let given_json = option_fixture();
    let mut mapped = VideoOptions::new();
    map_params(&video_create(given_json.clone()), &mut mapped);
    assert_eq!(Value::Object(mapped), given_json);
    let number_seconds = json!({"seconds": 4_i64});
    assert!(serde_json::from_value::<VideoCreate>(number_seconds).is_err());
    let number_character = json!({"characters": [{"id": 1_i64}]});
    assert!(serde_json::from_value::<VideoCreate>(number_character).is_err());
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
    let given = video_create(json!({"input_reference": "option-reference"}));
    let dropped = create_request(MODEL, PROMPT, API_BASE, &given, None);
    assert!(!dropped.fields.contains_key("input_reference"));
    assert!(dropped.files.is_empty());
}
