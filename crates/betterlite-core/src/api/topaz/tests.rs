#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageHeaders, ImageParams, ImageUsage,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::error::{TopazError, TopazException};
use super::params::{SUPPORTED_OPENAI_PARAMS, get_complete_url, map_openai_params};
use super::reply::{TopazTiming, transform_response};
use super::upload::{PreparedFile, TopazFile, prepare_file_tuple, transform_request};
use super::{
    DEFAULT_API_BASE, MODELS, TopazSettings, get_api_base, get_api_key, validate_environment,
};

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn settings(entries: &[(&str, &str)]) -> TopazSettings {
    TopazSettings::from_environment(&lookup(entries))
}

#[rstest]
#[case::unset(&[], None, DEFAULT_API_BASE)]
#[case::read_the_environment(
    &[
        ("TOPAZ_API_KEY", "server-key"),
        ("TOPAZ_API_BASE", "https://proxy.test/v1"),
    ],
    Some("server-key"),
    "https://proxy.test/v1"
)]
#[case::empty_variables_are_unset(
    &[("TOPAZ_API_KEY", ""), ("TOPAZ_API_BASE", "")],
    None,
    DEFAULT_API_BASE
)]
fn topaz_settings(
    #[case] entries: &[(&str, &str)],
    #[case] api_key: Option<&str>,
    #[case] api_base: &str,
) {
    let loaded = settings(entries);
    let found_key = loaded
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().to_owned());

    assert_eq!(found_key.as_deref(), api_key);
    assert_eq!(loaded.api_base, api_base);
}

#[test]
fn topaz_models_keep_their_pinned_names() {
    assert_eq!(
        MODELS,
        [
            "topaz/Standard V2",
            "topaz/Low Resolution V2",
            "topaz/CGI",
            "topaz/High Resolution V2",
            "topaz/Text Refine",
        ]
    );
}

#[rstest]
#[case::explicit_key(Some("explicit-key"), Some("server-key"), Some("explicit-key"))]
#[case::empty_explicit_key(Some(""), Some("server-key"), Some("server-key"))]
#[case::no_explicit_key(None, Some("server-key"), Some("server-key"))]
#[case::no_key(Some(""), None, None)]
fn topaz_api_key_takes_a_truthy_explicit_value_first(
    #[case] explicit: Option<&str>,
    #[case] configured: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let loaded = TopazSettings {
        api_key: configured.map(SecretString::from),
        api_base: DEFAULT_API_BASE.to_owned(),
    };
    let explicit = explicit.map(SecretString::from);
    let found = get_api_key(explicit.as_ref(), &loaded).map(|key| key.expose_secret().to_owned());

    assert_eq!(found.as_deref(), expected);
}

#[rstest]
#[case::explicit_base(
    Some("https://explicit.test"),
    "https://configured.test",
    "https://explicit.test"
)]
#[case::empty_explicit_base(Some(""), "https://configured.test", "https://configured.test")]
#[case::no_explicit_base(None, "https://configured.test", "https://configured.test")]
#[case::default_host(None, DEFAULT_API_BASE, DEFAULT_API_BASE)]
fn topaz_api_base_takes_a_truthy_explicit_value_first(
    #[case] explicit: Option<&str>,
    #[case] configured: &str,
    #[case] expected: &str,
) {
    let loaded = TopazSettings {
        api_key: None,
        api_base: configured.to_owned(),
    };

    assert_eq!(get_api_base(explicit, &loaded), expected);
}

#[test]
fn topaz_validate_environment_replaces_every_caller_header() {
    let key = SecretString::from("caller-key");

    let headers = validate_environment(Some(&key)).unwrap();

    assert_eq!(
        headers,
        ImageHeaders::from([
            ("Accept".to_owned(), "image/jpeg".to_owned()),
            ("X-API-Key".to_owned(), "caller-key".to_owned()),
        ])
    );
}

#[test]
fn topaz_validate_environment_passes_an_empty_key_through() {
    let key = SecretString::from("");

    let headers = validate_environment(Some(&key)).unwrap();

    assert_eq!(headers.get("X-API-Key").map(String::as_str), Some(""));
}

#[test]
fn topaz_validate_environment_rejects_a_missing_key() {
    assert_eq!(
        validate_environment(None).unwrap_err(),
        TopazError::MissingApiKey
    );
}

#[rstest]
#[case::default_host(None, "https://api.topazlabs.com/image/v1/enhance")]
#[case::given_base(
    Some("https://proxy.test/v1"),
    "https://proxy.test/v1/image/v1/enhance"
)]
#[case::kept_trailing_slash(
    Some("https://proxy.test/v1/"),
    "https://proxy.test/v1//image/v1/enhance"
)]
#[case::empty_base_is_the_default(Some(""), "https://api.topazlabs.com/image/v1/enhance")]
fn topaz_complete_url_keeps_every_slash(#[case] api_base: Option<&str>, #[case] expected: &str) {
    assert_eq!(get_complete_url(api_base), expected);
}

#[test]
fn topaz_supported_params_are_response_format_and_size() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["response_format", "size"]);
}

#[rstest]
#[case::url("url", "url")]
#[case::base64_value("b64_json", "b64_json")]
#[case::other_text("png", "png")]
fn topaz_map_openai_params_copies_response_format(
    #[case] response_format: &str,
    #[case] expected: &str,
) {
    let mapped = map_openai_params(
        &params(&[("response_format", text(response_format))]),
        ImageParams::new(),
    )
    .unwrap();

    assert_eq!(mapped, params(&[("output_format", text(expected))]));
}

#[rstest]
#[case::square("1024x1024", ("1024", "1024"))]
#[case::leading_zeros("007x99", ("007", "99"))]
#[case::empty_height("99x", ("99", ""))]
#[case::two_empty_parts("x", ("", ""))]
fn topaz_map_openai_params_splits_size_as_text(#[case] size: &str, #[case] parts: (&str, &str)) {
    let mapped = map_openai_params(&params(&[("size", text(size))]), ImageParams::new()).unwrap();

    assert_eq!(
        mapped,
        params(&[
            ("output_width", text(parts.0)),
            ("output_height", text(parts.1)),
        ])
    );
}

#[rstest]
#[case::one_part("1024")]
#[case::three_parts("1024x1024x2")]
#[case::empty("")]
fn topaz_map_openai_params_rejects_a_size_without_two_parts(#[case] size: &str) {
    let attempt = map_openai_params(&params(&[("size", text(size))]), ImageParams::new());

    assert_eq!(attempt, Err(TopazError::SizeFormat));
}

#[test]
fn topaz_map_openai_params_rejects_a_size_that_is_not_text() {
    let attempt = map_openai_params(
        &params(&[("size", Json::Number(Number::from(1024_i32)))]),
        ImageParams::new(),
    );

    assert_eq!(attempt, Err(TopazError::SizeFormat));
}

#[test]
fn topaz_map_openai_params_ignores_every_other_key() {
    let mapped = map_openai_params(
        &params(&[
            ("n", Json::Number(Number::from(2))),
            ("quality", text("hd")),
            ("model", text("topaz/CGI")),
        ]),
        ImageParams::new(),
    )
    .unwrap();

    assert_eq!(mapped, ImageParams::new());
}

#[test]
fn topaz_map_openai_params_overwrites_the_destination_keys() {
    let mapped = map_openai_params(
        &params(&[
            ("response_format", text("b64_json")),
            ("size", text("512x512")),
        ]),
        params(&[
            ("output_format", text("png")),
            ("output_width", text("1")),
            ("keep", text("me")),
        ]),
    )
    .unwrap();

    assert_eq!(
        mapped,
        params(&[
            ("output_format", text("b64_json")),
            ("output_width", text("512")),
            ("keep", text("me")),
            ("output_height", text("512")),
        ])
    );
}

#[test]
fn topaz_prepare_file_tuple_takes_bytes_as_content() {
    let prepared = prepare_file_tuple(TopazFile::Bytes(b"pixels"));

    assert_eq!(prepared.filename, "image.png");
    assert_eq!(prepared.content.as_deref(), Some(b"pixels".as_slice()));
    assert_eq!(prepared.content_type, "image/png");
    assert_eq!(prepared.headers, ImageHeaders::new());
}

#[test]
fn topaz_prepare_file_tuple_takes_the_tuple_members() {
    let headers = ImageHeaders::from([("x-part".to_owned(), "1".to_owned())]);

    let prepared = prepare_file_tuple(TopazFile::Headered {
        filename: Some("shot.png"),
        content: b"pixels",
        content_type: Some("image/jpeg"),
        headers: &headers,
    });

    assert_eq!(prepared.filename, "shot.png");
    assert_eq!(prepared.content.as_deref(), Some(b"pixels".as_slice()));
    assert_eq!(prepared.content_type, "image/jpeg");
    assert_eq!(prepared.headers, headers);
}

#[test]
fn topaz_prepare_file_tuple_falls_back_for_falsey_members() {
    let prepared = prepare_file_tuple(TopazFile::Typed {
        filename: Some(""),
        content: b"pixels",
        content_type: None,
    });

    assert_eq!(prepared.filename, "image.png");
    assert_eq!(prepared.content_type, "image/png");
}

#[test]
fn topaz_prepare_file_tuple_keeps_a_null_content_for_other_shapes() {
    let prepared = prepare_file_tuple(TopazFile::Unsupported);

    assert_eq!(prepared, PreparedFile::default());
    assert_eq!(prepared.content, None);
}

#[test]
fn topaz_typed_uploads_become_the_three_member_tuple() {
    let image = ImageFile {
        filename: Some("shot.png".to_owned()),
        content: b"pixels".to_vec(),
        content_type: Some("image/webp".to_owned()),
    };

    assert_eq!(
        TopazFile::from(&image),
        TopazFile::Typed {
            filename: Some("shot.png"),
            content: b"pixels",
            content_type: Some("image/webp"),
        }
    );
}

#[test]
fn topaz_transform_request_carries_one_image_and_the_mapped_data() {
    let data = params(&[("output_format", text("url"))]);

    let fields = transform_request(TopazFile::Bytes(b"pixels"), data.clone());

    assert_eq!(fields.file, prepare_file_tuple(TopazFile::Bytes(b"pixels")));
    assert_eq!(fields.data, data);
    assert_eq!(fields.data.get("model"), None);
}

#[test]
fn topaz_transform_response_base64_encodes_the_body() {
    let reply = transform_response(
        &[0xFF, 0xD8, 0xFF, 0x00, 0x10],
        TopazTiming::Elapsed(Duration::from_millis(2500)),
        17,
    );
    let image = reply.response.data.as_ref().unwrap().first().unwrap();

    assert_eq!(image.b64_json.as_deref(), Some("/9j/ABA="));
    assert_eq!(image.url, None);
    assert_eq!(image.revised_prompt, None);
    assert_eq!(reply.response.created, 17);
    assert_eq!(reply.response.data.as_ref().map(Vec::len), Some(1));
    assert_eq!(reply.response.usage, Some(ImageUsage::default()));
}

#[test]
fn topaz_reply_keeps_the_two_timing_sources_apart() {
    let sync = transform_response(
        b"pixels",
        TopazTiming::Elapsed(Duration::from_millis(2500)),
        1,
    );
    let asynchronous = transform_response(b"pixels", TopazTiming::Measured(1500.0), 1);

    assert_eq!(sync.response_ms, 2500.0);
    assert_eq!(asynchronous.response_ms, 1500.0);
    assert_eq!(sync.response, asynchronous.response);
}

#[test]
fn topaz_exception_keeps_the_upstream_reply() {
    let headers = ImageHeaders::from([("x-request-id".to_owned(), "abc".to_owned())]);

    let exception = TopazException::new(429, "rate limited".to_owned(), headers.clone());

    assert_eq!(exception.status_code, 429);
    assert_eq!(exception.message, "rate limited");
    assert_eq!(exception.headers, headers);
    assert_eq!(exception.to_string(), "rate limited");
}
