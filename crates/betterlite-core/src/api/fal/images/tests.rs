#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::decode::{
    FalImage, FalImageProviderSpecificFields, MAX_FAL_REPLY_BYTES, append_generation_images,
    decode_fal_image_reply, fal_images_to_image_objects,
};
use super::error::FalImageError;
use super::params::{GENERIC_OPENAI_PARAMS, image_generation_body, map_openai_params};
use super::settings::{DEFAULT_BASE_URL, FalImageSettings};

fn headers() -> ImageHeaders {
    BTreeMap::from([("x-request-id".to_owned(), "req-1".to_owned())])
}

fn url_image(url: &str) -> FalImage {
    FalImage {
        url: Some(url.to_owned()),
        b64_json: None,
        provider_specific_fields: None,
    }
}

fn provider(
    width: Option<u64>,
    height: Option<u64>,
    content_type: Option<&str>,
) -> Option<FalImageProviderSpecificFields> {
    Some(FalImageProviderSpecificFields {
        width,
        height,
        content_type: content_type.map(str::to_owned),
    })
}

fn decoded(body: &str) -> Vec<FalImage> {
    decode_fal_image_reply(200, &headers(), body.as_bytes()).expect("a decoded reply")
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

#[test]
fn fal_image_reply_reads_string_and_map_entries() {
    let images = decoded(
        r#"{"images": ["https://a.test/1.png", {"url": "https://b.test/2.png", "b64_json": "aGk=", "width": 10, "height": 20, "content_type": "image/png"}, {"url": 5, "b64_json": null}]}"#,
    );
    assert_eq!(
        images,
        vec![
            url_image("https://a.test/1.png"),
            FalImage {
                url: Some("https://b.test/2.png".to_owned()),
                b64_json: Some("aGk=".to_owned()),
                provider_specific_fields: provider(Some(10), Some(20), Some("image/png")),
            },
            FalImage {
                url: None,
                b64_json: None,
                provider_specific_fields: None,
            },
        ]
    );
}

#[test]
fn fal_image_reply_ignores_other_list_members() {
    let images = decoded(
        r#"{"images": [7, null, true, [], {}, {"url": "https://c.test/3.png"}, "https://d.test/4.png"]}"#,
    );
    assert_eq!(
        images,
        vec![
            FalImage {
                url: None,
                b64_json: None,
                provider_specific_fields: None,
            },
            url_image("https://c.test/3.png"),
            url_image("https://d.test/4.png"),
        ]
    );
}

#[rstest]
#[case::missing(r#"{}"#)]
#[case::object(r#"{"images": {"url": "https://a.test/1.png"}}"#)]
#[case::text(r#"{"images": "https://a.test/1.png"}"#)]
#[case::number(r#"{"images": 1}"#)]
#[case::null(r#"{"images": null}"#)]
fn fal_image_reply_requires_a_list(#[case] body: &str) {
    assert!(decoded(body).is_empty());
}

#[test]
fn fal_image_reply_keeps_only_positive_exact_dimension_integers() {
    let images = decoded(
        r#"{"images": [
            {"width": true, "height": 5},
            {"width": 0, "height": 5},
            {"width": -3, "height": 5},
            {"width": 1.5, "height": 5},
            {"width": "10", "height": 5},
            {"width": 12, "height": 7.0},
            {"width": 18446744073709551615, "height": 7}
        ]}"#,
    );
    let dimensions: Vec<(Option<u64>, Option<u64>)> = images
        .iter()
        .map(|image| {
            image
                .provider_specific_fields
                .as_ref()
                .map_or((None, None), |fields| (fields.width, fields.height))
        })
        .collect();
    assert_eq!(
        dimensions,
        vec![
            (None, Some(5)),
            (None, Some(5)),
            (None, Some(5)),
            (None, Some(5)),
            (None, Some(5)),
            (Some(12), None),
            (Some(u64::MAX), Some(7)),
        ]
    );
}

#[test]
fn fal_image_reply_keeps_an_empty_content_type() {
    let images = decoded(r#"{"images": [{"content_type": ""}]}"#);
    assert_eq!(
        images[0].provider_specific_fields,
        provider(None, None, Some(""))
    );
}

#[test]
fn fal_image_reply_omits_empty_provider_fields() {
    let images = decoded(r#"{"images": [{"url": "https://a.test/1.png"}]}"#);
    assert_eq!(images[0].provider_specific_fields, None);
}

#[test]
fn fal_image_reply_rejects_invalid_json_with_status_and_headers() {
    match decode_fal_image_reply(502, &headers(), b"{") {
        Err(FalImageError::ReplyDecode {
            status,
            headers: carried,
            ..
        }) => {
            assert_eq!(status, 502);
            assert_eq!(carried, headers());
        }
        other => panic!("{other:?}"),
    }
}

#[rstest]
#[case::list(b"[]")]
#[case::text(br#""images""#)]
#[case::number(b"7")]
#[case::null(b"null")]
fn fal_image_reply_rejects_a_non_object_root(#[case] body: &[u8]) {
    match decode_fal_image_reply(500, &headers(), body) {
        Err(FalImageError::ReplyNotAnObject {
            status,
            headers: carried,
        }) => {
            assert_eq!(status, 500);
            assert_eq!(carried, headers());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn fal_image_reply_rejects_an_oversized_reply() {
    let oversized = vec![b' '; MAX_FAL_REPLY_BYTES + 1];
    match decode_fal_image_reply(200, &headers(), &oversized) {
        Err(FalImageError::ReplyTooLarge {
            status,
            headers: carried,
            limit,
        }) => {
            assert_eq!(status, 200);
            assert_eq!(carried, headers());
            assert_eq!(limit, MAX_FAL_REPLY_BYTES);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn fal_image_reply_accepts_a_reply_at_the_bound() {
    let mut at_bound = vec![b' '; MAX_FAL_REPLY_BYTES];
    at_bound[..13].copy_from_slice(br#"{"images":[]}"#);
    let images = decode_fal_image_reply(200, &headers(), &at_bound).expect("a decoded reply");
    assert!(images.is_empty());
}

#[test]
fn fal_images_to_image_objects_requires_a_list() {
    let object = Json::Object(IndexMap::from([(
        "url".to_owned(),
        Json::Text("https://a.test/1.png".to_owned()),
    )]));
    assert!(fal_images_to_image_objects(&object).is_empty());
}

#[test]
fn generation_appends_to_existing_data() {
    let mut data = Some(vec![url_image("https://first.test/1.png")]);
    append_generation_images(
        &mut data,
        200,
        &headers(),
        br#"{"images": ["https://second.test/2.png"]}"#,
    )
    .expect("appended images");
    assert_eq!(
        data,
        Some(vec![
            url_image("https://first.test/1.png"),
            url_image("https://second.test/2.png"),
        ])
    );
}

#[test]
fn generation_starts_an_empty_data_list() {
    let mut data = None;
    append_generation_images(&mut data, 200, &headers(), br#"{"images": []}"#)
        .expect("an empty reply");
    assert_eq!(data, Some(Vec::new()));
}

#[test]
fn generation_keeps_existing_data_on_a_decode_error() {
    let mut data = Some(vec![url_image("https://first.test/1.png")]);
    let error = append_generation_images(&mut data, 502, &headers(), b"{");
    assert!(error.is_err());
    assert_eq!(data, Some(vec![url_image("https://first.test/1.png")]));
    let mut empty = None;
    assert!(append_generation_images(&mut empty, 502, &headers(), b"{").is_err());
    assert_eq!(empty, None);
}

#[test]
fn generic_supported_params_are_n_response_format_and_size() {
    assert_eq!(GENERIC_OPENAI_PARAMS, ["n", "response_format", "size"]);
}

#[test]
fn generic_mapping_copies_supported_params() {
    let non_default = params(&[
        ("n", number(2)),
        ("response_format", Json::Text("url".to_owned())),
        ("size", Json::Text("1024x768".to_owned())),
    ]);
    let mapped = map_openai_params(&non_default, ImageParams::new(), "fal-ai/flux", false)
        .expect("supported params");
    assert_eq!(mapped, non_default);
}

#[test]
fn generic_mapping_keeps_an_existing_optional_param() {
    let non_default = params(&[("n", number(2))]);
    let optional = params(&[("n", number(4))]);
    let mapped =
        map_openai_params(&non_default, optional, "fal-ai/flux", false).expect("an existing param");
    assert_eq!(mapped.get("n"), Some(&number(4)));
}

#[test]
fn generic_mapping_rejects_an_unsupported_param() {
    let non_default = params(&[("seed", number(1))]);
    match map_openai_params(&non_default, ImageParams::new(), "fal-ai/flux", false) {
        Err(FalImageError::UnsupportedParameters { parameters, model }) => {
            assert_eq!(parameters, ["seed"]);
            assert_eq!(model, "fal-ai/flux");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn generic_mapping_drops_unsupported_params_with_drop_params() {
    let non_default = params(&[("seed", number(1)), ("n", number(2))]);
    let mapped =
        map_openai_params(&non_default, ImageParams::new(), "fal-ai/flux", true).expect("dropped");
    assert_eq!(mapped, params(&[("n", number(2))]));
}

#[test]
fn generic_mapping_keeps_an_unsupported_param_that_is_already_present() {
    let non_default = params(&[("seed", number(1))]);
    let optional = params(&[("seed", number(9))]);
    let mapped =
        map_openai_params(&non_default, optional, "fal-ai/flux", false).expect("an existing param");
    assert_eq!(mapped.get("seed"), Some(&number(9)));
}

#[test]
fn generic_body_puts_provider_options_over_the_prompt() {
    let optional = params(&[("prompt", Json::Text("a dog".to_owned())), ("n", number(2))]);
    let body = image_generation_body("a cat", optional);
    assert_eq!(
        body,
        params(&[("prompt", Json::Text("a dog".to_owned())), ("n", number(2)),])
    );
}

#[test]
fn generic_body_starts_with_the_prompt() {
    let body = image_generation_body("a cat", ImageParams::new());
    assert_eq!(body, params(&[("prompt", Json::Text("a cat".to_owned()))]));
}

#[test]
fn complete_url_posts_a_generic_model_to_the_base_itself() {
    let settings = FalImageSettings::default();
    assert_eq!(settings.complete_url(None, ""), DEFAULT_BASE_URL);
    assert_eq!(
        settings.complete_url(Some("https://proxy.test/fal///"), ""),
        "https://proxy.test/fal"
    );
}

#[test]
fn complete_url_uses_an_explicit_base_before_the_environment_base() {
    let settings = FalImageSettings {
        api_key: None,
        api_base: "https://environment.test/fal/".to_owned(),
    };
    assert_eq!(
        settings.complete_url(Some("https://explicit.test/fal"), "endpoint"),
        "https://explicit.test/fal/endpoint"
    );
    assert_eq!(
        settings.complete_url(Some(""), "endpoint"),
        "https://environment.test/fal/endpoint"
    );
    assert_eq!(
        settings.complete_url(None, ""),
        "https://environment.test/fal"
    );
}

#[test]
fn authorize_requires_a_key() {
    let settings = FalImageSettings::default();
    assert!(matches!(
        settings.authorize(&headers(), None),
        Err(FalImageError::MissingApiKey)
    ));
    let empty = SecretString::from("");
    assert!(matches!(
        settings.authorize(&headers(), Some(&empty)),
        Err(FalImageError::MissingApiKey)
    ));
}

#[test]
fn authorize_prefers_an_explicit_key_and_overwrites_authorization() {
    let settings = FalImageSettings {
        api_key: Some(SecretString::from("environment-key")),
        api_base: DEFAULT_BASE_URL.to_owned(),
    };
    let mut supplied = headers();
    supplied.insert("Authorization".to_owned(), "Bearer other".to_owned());
    let explicit = settings
        .authorize(&supplied, Some(&SecretString::from("explicit-key")))
        .expect("an explicit key");
    assert_eq!(
        explicit.get("Authorization").map(String::as_str),
        Some("Key explicit-key")
    );
    assert_eq!(
        explicit.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
    let from_environment = settings
        .authorize(&supplied, None)
        .expect("an environment key");
    assert_eq!(
        from_environment.get("Authorization").map(String::as_str),
        Some("Key environment-key")
    );
    let empty_explicit = SecretString::from("");
    let falls_back = settings
        .authorize(&supplied, Some(&empty_explicit))
        .expect("the environment key");
    assert_eq!(
        falls_back.get("Authorization").map(String::as_str),
        Some("Key environment-key")
    );
}

#[rstest]
fn replays_the_fal_image_reply_corpus(#[files("fuzz/corpus/fal_image_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus file");
    drop(decode_fal_image_reply(200, &ImageHeaders::new(), &bytes));
}
