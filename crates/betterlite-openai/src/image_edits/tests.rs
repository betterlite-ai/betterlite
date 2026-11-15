#![cfg(test)]

use betterlite_config::sections::openai_credentials::{DEFAULT_API_BASE, Settings};
use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::environment::{complete_url, validate_environment};
use super::error::{BAD_REQUEST_STATUS, ImageEditError};
use super::family::{
    ImageEditFamily, SUPPORTED_OPENAI_PARAMS, map_openai_params, select_image_edit_family,
};
use super::file::{
    DEFAULT_CONTENT_TYPE, DEFAULT_IMAGE_NAME, DEFAULT_MASK_NAME, ImageEditFile, multipart_parts,
};
use super::request::{ImageEditForm, ImageEditRequest, transform_image_edit_request};
use super::response::transform_image_edit_response;
use crate::api::image_contracts::records::ImageParams;
use crate::credentials::{
    ExplicitCredentials, LiteLlmGlobals, OpenAiCredentials, get_openai_credentials,
    resolve_openai_api_base,
};
use crate::openai_base_config::Headers;

const NOW: i64 = 1_700_000_000;
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nimage";
const JPEG: &[u8] = b"\xff\xd8\xffjpeg";
const SUCCESS_BODY: &str = r#"{"created":1700000000,"data":[{"b64_json":"YQ=="}]}"#;

fn supplied(content: &[u8], name: Option<&str>) -> ImageEditFile {
    ImageEditFile {
        content: content.to_vec(),
        name: name.map(str::to_owned),
    }
}

fn options(source: &str) -> ImageParams {
    serde_json::from_str::<IndexMap<String, Json>>(source).expect("options")
}

fn request(
    model: &str,
    prompt: Option<&str>,
    image: Option<Vec<Option<ImageEditFile>>>,
    mask: Option<Vec<Option<ImageEditFile>>>,
    options: ImageParams,
) -> ImageEditRequest {
    ImageEditRequest {
        model: model.to_owned(),
        prompt: prompt.map(str::to_owned),
        image,
        mask,
        options,
    }
}

fn form(family: ImageEditFamily, request: &ImageEditRequest) -> ImageEditForm {
    transform_image_edit_request(family, request).expect("form")
}

fn filenames(found: &ImageEditForm) -> Vec<(String, Option<String>, Option<String>)> {
    found
        .files
        .iter()
        .map(|(field, file)| {
            (
                field.clone(),
                file.filename.clone(),
                file.content_type.clone(),
            )
        })
        .collect()
}

fn pair(
    field: &str,
    filename: &str,
    content_type: &str,
) -> (String, Option<String>, Option<String>) {
    (
        field.to_owned(),
        Some(filename.to_owned()),
        Some(content_type.to_owned()),
    )
}

fn authorization(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get("authorization")?;
    value.to_str().ok()
}

fn credentials(key: Option<&str>) -> OpenAiCredentials {
    OpenAiCredentials {
        api_base: String::from(DEFAULT_API_BASE),
        api_key: key.map(SecretString::from),
        organization: None,
    }
}

fn endpoint_settings() -> Settings {
    Settings {
        api_base: String::from("https://env.test/v1"),
        organization: None,
    }
}

#[rstest]
#[case::single_image_and_mask(0)]
#[case::multiple_image_array(1)]
#[case::dalle2_fields(2)]
#[case::gpt_image_optional_fields(3)]
#[case::filename_and_mime(4)]
#[case::inline_data_edit(5)]
#[case::edit_auth_and_endpoint(6)]
#[case::edit_reply_error(7)]
fn image_edits_cases(#[case] case: u8) {
    match case {
        0 => single_image_and_mask(),
        1 => multiple_image_array(),
        2 => dalle2_fields(),
        3 => gpt_image_optional_fields(),
        4 => filename_and_mime(),
        5 => inline_data_edit(),
        6 => edit_auth_and_endpoint(),
        7 => edit_reply_error(),
        _ => panic!("unknown case {case}"),
    }
}

fn single_image_and_mask() {
    let source = request(
        "gpt-image-1",
        Some("change"),
        Some(vec![Some(supplied(PNG, Some("baby.png")))]),
        Some(vec![Some(supplied(PNG, Some("mask-src.png")))]),
        options(r#"{"n":2}"#),
    );
    let found = form(ImageEditFamily::OpenAi, &source);
    assert_eq!(
        found.data,
        options(r#"{"model":"gpt-image-1","n":2,"prompt":"change"}"#)
    );
    assert_eq!(
        filenames(&found),
        vec![
            pair("image[]", "baby.png", "image/png"),
            pair("mask", "mask-src.png", "image/png"),
        ]
    );
    assert_eq!(found.files[0].1.content, PNG);
    assert_eq!(found.files[1].1.content, PNG);
}

fn multiple_image_array() {
    let source = request(
        "gpt-image-1",
        None,
        Some(vec![
            Some(supplied(PNG, None)),
            None,
            Some(supplied(JPEG, Some("b.jpg"))),
        ]),
        Some(vec![
            Some(supplied(PNG, Some("m1.png"))),
            Some(supplied(JPEG, None)),
        ]),
        ImageParams::new(),
    );
    let found = form(ImageEditFamily::OpenAi, &source);
    assert_eq!(
        filenames(&found),
        vec![
            pair("image[]", DEFAULT_IMAGE_NAME, "image/png"),
            pair("image[]", "b.jpg", "image/jpeg"),
            pair("mask", "m1.png", "image/png"),
        ]
    );
    let empty = form(
        ImageEditFamily::OpenAi,
        &request(
            "gpt-image-1",
            None,
            Some(vec![Some(supplied(PNG, None))]),
            Some(Vec::new()),
            ImageParams::new(),
        ),
    );
    assert_eq!(
        filenames(&empty),
        vec![pair("image[]", DEFAULT_IMAGE_NAME, "image/png")]
    );
}

fn dalle2_fields() {
    family_selection();
    dalle2_single_image();
    dalle2_rejects_two_images();
}

fn family_selection() {
    assert_eq!(
        select_image_edit_family("dall-e-2"),
        ImageEditFamily::DallE2
    );
    assert_eq!(
        select_image_edit_family("DALL-E-2"),
        ImageEditFamily::DallE2
    );
    assert_eq!(
        select_image_edit_family("dall_e_2"),
        ImageEditFamily::DallE2
    );
    assert_eq!(
        select_image_edit_family("dall-e-2-preview"),
        ImageEditFamily::OpenAi
    );
    assert_eq!(
        select_image_edit_family("gpt-image-1"),
        ImageEditFamily::OpenAi
    );
    assert_eq!(
        select_image_edit_family("gpt_image_1"),
        ImageEditFamily::OpenAi
    );
    assert_eq!(ImageEditFamily::DallE2.name(), "dall_e_2");
    assert_eq!(ImageEditFamily::OpenAi.name(), "openai");
}

fn dalle2_single_image() {
    let single = form(
        ImageEditFamily::DallE2,
        &request(
            "dall-e-2",
            Some("change"),
            Some(vec![Some(supplied(PNG, None))]),
            None,
            ImageParams::new(),
        ),
    );
    assert_eq!(
        filenames(&single),
        vec![pair("image", DEFAULT_IMAGE_NAME, "image/png")]
    );
}

fn dalle2_rejects_two_images() {
    let rejected = transform_image_edit_request(
        ImageEditFamily::DallE2,
        &request(
            "dall-e-2",
            Some("change"),
            Some(vec![Some(supplied(PNG, None)), None]),
            None,
            ImageParams::new(),
        ),
    );
    let error = rejected.expect_err("two entries");
    assert_eq!(error.status(), Some(BAD_REQUEST_STATUS));
    assert!(matches!(error, ImageEditError::MultipleImages));
    assert_eq!(
        format!("{error}"),
        "DALL-E-2 only supports editing a single image. Please provide one image."
    );
}

fn gpt_image_optional_fields() {
    supported_params();
    let supplied_options = options(
        r#"{"background":"transparent","quality":"high","prompt":"from-options","image":null,"mask":null,"extension":3,"model":"other"}"#,
    );
    mapped_options(&supplied_options);
    option_fields_override(supplied_options);
}

fn supported_params() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "image",
            "prompt",
            "background",
            "input_fidelity",
            "mask",
            "model",
            "n",
            "quality",
            "response_format",
            "size",
            "user",
            "extra_headers",
            "extra_query",
            "extra_body",
            "timeout",
        ]
    );
}

fn mapped_options(supplied_options: &ImageParams) {
    let mapped = map_openai_params(supplied_options);
    assert_eq!(&mapped, supplied_options);
    assert_eq!(mapped.len(), supplied_options.len());
}

fn option_fields_override(supplied_options: ImageParams) {
    let found = form(
        ImageEditFamily::OpenAi,
        &request(
            "gpt-image-1",
            Some("change"),
            Some(vec![Some(supplied(PNG, None))]),
            None,
            supplied_options,
        ),
    );
    assert_eq!(
        found.data,
        options(
            r#"{"model":"other","background":"transparent","quality":"high","prompt":"change","extension":3}"#
        )
    );
    assert_eq!(
        filenames(&found),
        vec![pair("image[]", DEFAULT_IMAGE_NAME, "image/png")]
    );
}

fn filename_and_mime() {
    named_file_and_mask();
    literal_request_bytes();
    unknown_image_bytes();
}

fn named_file_and_mask() {
    let named = form(
        ImageEditFamily::OpenAi,
        &request(
            "gpt-image-1",
            None,
            Some(vec![Some(supplied(JPEG, Some("photo.jpeg")))]),
            Some(vec![Some(supplied(JPEG, None))]),
            ImageParams::new(),
        ),
    );
    assert_eq!(
        filenames(&named),
        vec![
            pair("image[]", "photo.jpeg", "image/jpeg"),
            pair("mask", DEFAULT_MASK_NAME, "image/jpeg"),
        ]
    );
}

fn literal_request_bytes() {
    // a request string never authorizes a filesystem read; its bytes are what LiteLLM uploads
    let path = b"/etc/passwd";
    let literal = form(
        ImageEditFamily::OpenAi,
        &request(
            "gpt-image-1",
            None,
            Some(vec![Some(supplied(path, None))]),
            None,
            ImageParams::new(),
        ),
    );
    assert_eq!(literal.files[0].1.content, path.to_vec());
    assert_eq!(
        filenames(&literal),
        vec![pair("image[]", DEFAULT_IMAGE_NAME, DEFAULT_CONTENT_TYPE)]
    );
}

fn unknown_image_bytes() {
    let unknown = form(
        ImageEditFamily::OpenAi,
        &request(
            "gpt-image-1",
            None,
            Some(vec![Some(supplied(b"not an image", None))]),
            None,
            ImageParams::new(),
        ),
    );
    assert_eq!(
        filenames(&unknown),
        vec![pair("image[]", DEFAULT_IMAGE_NAME, DEFAULT_CONTENT_TYPE)]
    );
}

fn inline_data_edit() {
    let found = form(
        ImageEditFamily::OpenAi,
        &request(
            "gpt-image-1",
            Some("change"),
            Some(vec![Some(supplied(JPEG, None))]),
            None,
            ImageParams::new(),
        ),
    );
    assert_eq!(
        found.data,
        options(r#"{"model":"gpt-image-1","prompt":"change"}"#)
    );
    let parts = multipart_parts(&found.files);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].name, "image[]");
    assert_eq!(parts[0].file_name.as_deref(), Some(DEFAULT_IMAGE_NAME));
    assert_eq!(parts[0].content_type.as_deref(), Some("image/jpeg"));
    assert_eq!(parts[0].data, JPEG);
}

fn edit_auth_and_endpoint() {
    explicit_key_header();
    global_and_openai_key_header();
    inherited_key_header();
    base_and_endpoint_forms();
}

fn explicit_key_header() {
    let explicit = get_openai_credentials(
        ExplicitCredentials {
            api_base: Some(String::from("https://explicit.test/v1")),
            api_key: Some(SecretString::from("explicit-key")),
            organization: None,
        },
        LiteLlmGlobals::default(),
        &endpoint_settings(),
        Some(SecretString::from("env-key")),
    );
    assert_eq!(explicit.api_base, "https://explicit.test/v1");
    let headers = validate_environment(&Headers::new(), &explicit).expect("headers");
    assert_eq!(authorization(&headers), Some("Bearer explicit-key"));
}

fn global_and_openai_key_header() {
    let global = get_openai_credentials(
        ExplicitCredentials::default(),
        LiteLlmGlobals {
            api_base: Some(String::from("https://global.test/v1")),
            api_key: Some(SecretString::from("global-key")),
            openai_key: Some(SecretString::from("openai-key")),
            organization: None,
        },
        &endpoint_settings(),
        Some(SecretString::from("env-key")),
    );
    assert_eq!(global.api_base, "https://global.test/v1");
    let openai_key = get_openai_credentials(
        ExplicitCredentials::default(),
        LiteLlmGlobals {
            api_base: None,
            api_key: None,
            openai_key: Some(SecretString::from("openai-key")),
            organization: None,
        },
        &endpoint_settings(),
        Some(SecretString::from("env-key")),
    );
    let headers = validate_environment(&Headers::new(), &openai_key).expect("headers");
    assert_eq!(authorization(&headers), Some("Bearer openai-key"));
}

fn inherited_key_header() {
    let inherited = validate_environment(&Headers::new(), &credentials(None)).expect("headers");
    assert_eq!(authorization(&inherited), Some("Bearer None"));
}

fn base_and_endpoint_forms() {
    let settings = endpoint_settings();
    assert_eq!(
        resolve_openai_api_base(None, None, &settings),
        "https://env.test/v1"
    );
    assert_eq!(
        complete_url("https://slash.test/v1///")
            .expect("url")
            .as_str(),
        "https://slash.test/v1/images/edits"
    );
    assert_eq!(
        complete_url(DEFAULT_API_BASE).expect("url").as_str(),
        "https://api.openai.com/v1/images/edits"
    );
}

fn edit_reply_error() {
    let reply = transform_image_edit_response(200, SUCCESS_BODY.as_bytes(), NOW).expect("reply");
    assert_eq!(reply.created, NOW);
    assert_eq!(reply.data.len(), 1);
    let missing = transform_image_edit_response(200, b"not json", NOW).expect_err("not json");
    assert_eq!(missing.status(), Some(200));
    assert!(matches!(missing, ImageEditError::OpenAi(_)));
    assert_eq!(format!("{missing}"), "not json");
    // httpx decodes raw_response.text with errors="replace", so invalid bytes arrive as U+FFFD
    let invalid = transform_image_edit_response(200, b"not \xff json", NOW).expect_err("bytes");
    assert_eq!(format!("{invalid}"), "not \u{fffd} json");
    let shape =
        transform_image_edit_response(200, br#"{"created":"nope"}"#, NOW).expect_err("wrong shape");
    assert_eq!(shape.status(), None);
    assert!(matches!(shape, ImageEditError::Reply(_)));
    // LiteLLM builds an empty ImageResponse from an error object; the shared decoder reports it instead
    let objected = transform_image_edit_response(200, br#"{"error":{"message":"boom"}}"#, NOW)
        .expect_err("object");
    assert_eq!(objected.status(), Some(422));
    assert!(matches!(objected, ImageEditError::Reply(_)));
}

#[test]
fn openai_image_edits_credential_identity_isolation() {
    let first = validate_environment(&Headers::new(), &credentials(Some("sk-identity-a")))
        .expect("headers");
    let second = validate_environment(&Headers::new(), &credentials(Some("sk-identity-b")))
        .expect("headers");
    assert_eq!(authorization(&first), Some("Bearer sk-identity-a"));
    assert_eq!(authorization(&second), Some("Bearer sk-identity-b"));
    assert_ne!(authorization(&first), authorization(&second));
    let mut supplied = Headers::new();
    supplied.insert(String::from("x-team"), String::from("a"));
    let team =
        validate_environment(&supplied, &credentials(Some("sk-identity-a"))).expect("headers");
    assert_eq!(
        team.get("x-team")
            .map(|value| value.to_str().expect("text")),
        Some("a")
    );
    assert!(second.get("x-team").is_none());
    let again =
        validate_environment(&Headers::new(), &credentials(Some("sk-identity-a"))).expect("again");
    assert_eq!(authorization(&again), authorization(&first));
}

#[test]
fn openai_image_edits_secret_safe_failure() {
    const SECRET: &str = "sk-fixture-secret";
    let headers =
        validate_environment(&Headers::new(), &credentials(Some(SECRET))).expect("headers");
    let bearer = format!("Bearer {SECRET}");
    assert_eq!(authorization(&headers), Some(bearer.as_str()));
    let invalid = complete_url("http://[::1sk-fixture").expect_err("bad base");
    let multiple = transform_image_edit_request(
        ImageEditFamily::DallE2,
        &request(
            "dall-e-2",
            None,
            Some(vec![Some(supplied(PNG, None)), Some(supplied(PNG, None))]),
            None,
            ImageParams::new(),
        ),
    )
    .expect_err("two images");
    let shape = transform_image_edit_response(200, b"not json", NOW).expect_err("body");
    let shown =
        format!("{headers:?} {invalid:?} {invalid} {multiple:?} {multiple} {shape:?} {shape}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[test]
fn complete_url_rejects_an_unparsable_base() {
    let error = complete_url("not a url").expect_err("invalid");
    assert_eq!(error.status(), None);
    assert_eq!(format!("{error}"), "the api_base is not a valid URL");
}

#[test]
fn validate_environment_keeps_supplied_headers_and_overwrites_authorization() {
    let mut supplied = Headers::new();
    supplied.insert(String::from("authorization"), String::from("Bearer stale"));
    supplied.insert(String::from("x-extra"), String::from("kept"));
    let headers = validate_environment(&supplied, &credentials(Some("fresh"))).expect("headers");
    assert_eq!(authorization(&headers), Some("Bearer fresh"));
    assert_eq!(
        headers
            .get("x-extra")
            .map(|value| value.to_str().expect("text")),
        Some("kept")
    );
    assert!(
        headers
            .get("authorization")
            .is_some_and(HeaderValue::is_sensitive)
    );
}
