#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use betterlite_config::sections::Sections;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::reply::ImageDatum;
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::{
    GenerationConfig, SUPPORTED_OPENAI_PARAMS, apply_image_response, complete_url,
    generation_endpoint, map_openai_params, request_body, validate_environment,
};
use crate::api::bfl::common::{
    BflError, BflReply, DEFAULT_MAX_POLLING_TIME, DEFAULT_POLLING_INTERVAL, IMAGE_EDIT_MODELS,
    IMAGE_GENERATION_MODELS, MAX_REPLY_BYTES, check_bfl_poll_url, decode_bfl_reply,
};

fn settings(entries: &[(&str, &str)]) -> (Option<SecretString>, String) {
    let mut found = BTreeMap::new();
    for (name, value) in entries {
        found.insert((*name).to_owned(), (*value).to_owned());
    }
    Sections::bfl_generation_settings(&found)
}

fn params(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut found = IndexMap::new();
    for (name, value) in entries {
        found.insert((*name).to_owned(), value.clone());
    }
    found
}

fn reply(body: &str) -> BflReply {
    decode_bfl_reply(body.as_bytes()).unwrap()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn existing_image() -> ImageDatum {
    ImageDatum {
        b64_json: None,
        url: Some("https://delivery.test/old.png".to_owned()),
        revised_prompt: None,
        provider_specific_fields: None,
    }
}

fn urls(reply: &BflReply, existing: Vec<ImageDatum>) -> Vec<Option<String>> {
    apply_image_response(reply, existing, 0)
        .unwrap()
        .data
        .into_iter()
        .map(|datum| datum.url)
        .collect()
}

#[test]
fn constants_match_the_source() {
    assert_eq!(DEFAULT_POLLING_INTERVAL, Duration::from_millis(1500));
    assert_eq!(DEFAULT_MAX_POLLING_TIME, Duration::from_secs(300));
    assert_eq!(
        IMAGE_GENERATION_MODELS.map(|(model, _)| model),
        [
            "flux-pro-1.1",
            "flux-pro-1.1-ultra",
            "flux-dev",
            "flux-pro",
            "flux-kontext-pro",
            "flux-kontext-max"
        ]
    );
    assert_eq!(IMAGE_GENERATION_MODELS[1].1, "/v1/flux-pro-1.1-ultra");
    assert_eq!(
        IMAGE_EDIT_MODELS.map(|(model, _)| model),
        [
            "flux-kontext-pro",
            "flux-kontext-max",
            "flux-pro-1.0-fill",
            "flux-pro-1.0-expand"
        ]
    );
    assert_eq!(IMAGE_EDIT_MODELS[2].1, "/v1/flux-pro-1.0-fill");
}

#[test]
fn supported_params_are_the_source_list() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "n",
            "size",
            "quality",
            "seed",
            "output_format",
            "safety_tolerance",
            "prompt_upsampling",
            "raw",
            "num_images",
            "image_url",
            "image_prompt_strength",
            "aspect_ratio"
        ]
    );
}

#[rstest]
#[case::pro("flux-pro-1.1", "/v1/flux-pro-1.1")]
#[case::ultra("flux-pro-1.1-ultra", "/v1/flux-pro-1.1-ultra")]
#[case::dev("flux-dev", "/v1/flux-dev")]
#[case::pro_legacy("flux-pro", "/v1/flux-pro")]
#[case::kontext_pro("flux-kontext-pro", "/v1/flux-kontext-pro")]
#[case::kontext_max("flux-kontext-max", "/v1/flux-kontext-max")]
#[case::upper_case("black_forest_labs/FLUX-Pro-1.1-ULTRA", "/v1/flux-pro-1.1-ultra")]
#[case::nested_prefix("proxy/bfl/flux-dev", "/v1/flux-dev")]
fn resolves_the_six_generation_models(#[case] model: &str, #[case] endpoint: &str) {
    assert_eq!(generation_endpoint(model).unwrap(), endpoint);
    assert_eq!(
        GenerationConfig::for_model(model).unwrap(),
        GenerationConfig
    );
}

#[rstest]
#[case::unknown("flux-unknown")]
#[case::empty("")]
#[case::only_prefix("black_forest_labs/")]
fn rejects_other_generation_models(#[case] model: &str) {
    assert!(matches!(
        generation_endpoint(model),
        Err(BflError::UnknownModel { .. })
    ));
    assert!(GenerationConfig::for_model(model).is_err());
}

#[test]
fn unknown_model_error_names_the_lowercased_final_component() {
    let error = generation_endpoint("black_forest_labs/FLUX-Unknown").unwrap_err();

    assert!(matches!(&error, BflError::UnknownModel { model } if model == "flux-unknown"));
    assert_eq!(error.status(), None);
}

#[test]
fn ultra_names_turn_n_into_num_images() {
    let mapped = map_openai_params(
        &params(&[("n", number(2))]),
        IndexMap::new(),
        "flux-pro-1.1-ultra",
        false,
    )
    .unwrap();

    assert_eq!(mapped, params(&[("num_images", number(2))]));
}

#[test]
fn other_models_drop_n() {
    let mapped = map_openai_params(
        &params(&[("n", number(2))]),
        IndexMap::new(),
        "flux-pro-1.1",
        false,
    )
    .unwrap();

    assert!(mapped.is_empty());
}

#[rstest]
#[case::ultra_hd("flux-pro-1.1-ultra", Some(true))]
#[case::other_model_hd("flux-dev", None)]
#[case::ultra_standard("flux-pro-1.1-ultra", None)]
fn quality_hd_becomes_raw_only_for_ultra(#[case] model: &str, #[case] raw: Option<bool>) {
    let mapped = map_openai_params(
        &params(&[("quality", text("hd"))]),
        IndexMap::new(),
        model,
        false,
    )
    .unwrap();

    assert_eq!(mapped.get("raw").cloned(), raw.map(Json::Bool));
    assert!(!mapped.contains_key("quality"));
}

#[test]
fn other_quality_values_disappear() {
    let mapped = map_openai_params(
        &params(&[("quality", text("standard"))]),
        IndexMap::new(),
        "flux-pro-1.1-ultra",
        false,
    )
    .unwrap();

    assert!(mapped.is_empty());
}

#[rstest]
#[case::table_hit("1792x1024", Some((1792, 1024)))]
#[case::table_hit_small("256x256", Some((256, 256)))]
#[case::custom("1344X768", None)]
#[case::custom_lower("1344x768", Some((1344, 768)))]
#[case::spaced_around(" 1344x768 ", Some((1344, 768)))]
#[case::no_x("1344", None)]
fn maps_lowercase_x_sizes_into_dimensions(
    #[case] size: &str,
    #[case] dimensions: Option<(i64, i64)>,
) {
    let mapped = map_openai_params(
        &params(&[("size", text(size))]),
        IndexMap::new(),
        "flux-dev",
        false,
    )
    .unwrap();

    match dimensions {
        Some((width, height)) => {
            assert_eq!(mapped.get("width"), Some(&number(width)));
            assert_eq!(mapped.get("height"), Some(&number(height)));
        }
        None => assert!(mapped.is_empty()),
    }
}

#[rstest]
#[case::letters("1024xwide")]
#[case::three_parts("1024x768x2")]
#[case::empty("x512")]
#[case::trailing("1344x")]
fn malformed_sizes_raise_the_parse_error(#[case] size: &str) {
    let error = map_openai_params(
        &params(&[("size", text(size))]),
        IndexMap::new(),
        "flux-dev",
        false,
    )
    .unwrap_err();

    assert!(matches!(&error, BflError::InvalidSize { size: found, .. } if found == size));
    assert!(std::error::Error::source(&error).is_some());
    assert_eq!(error.status(), None);
}

#[test]
fn null_size_is_skipped_and_a_wrong_type_is_named() {
    let skipped = map_openai_params(
        &params(&[("size", Json::Null)]),
        IndexMap::new(),
        "flux-dev",
        false,
    )
    .unwrap();
    let wrong = map_openai_params(
        &params(&[("size", number(3))]),
        IndexMap::new(),
        "flux-dev",
        false,
    )
    .unwrap_err();

    assert!(skipped.is_empty());
    assert!(matches!(wrong, BflError::InvalidSizeField));
}

#[test]
fn an_existing_optional_key_wins_before_mapping() {
    let mapped = map_openai_params(
        &params(&[("size", text("1024x1024")), ("n", number(4))]),
        params(&[("size", text("512x512")), ("num_images", number(3))]),
        "flux-pro-1.1-ultra",
        false,
    )
    .unwrap();

    assert_eq!(
        mapped,
        params(&[("size", text("512x512")), ("num_images", number(3))])
    );
}

#[test]
fn unsupported_parameters_fail_unless_dropped() {
    let kept = map_openai_params(
        &params(&[("style", text("natural"))]),
        IndexMap::new(),
        "flux-dev",
        false,
    )
    .unwrap_err();
    let dropped = map_openai_params(
        &params(&[("style", text("natural"))]),
        IndexMap::new(),
        "flux-dev",
        true,
    )
    .unwrap();

    assert!(matches!(kept, BflError::UnsupportedParameter { .. }));
    assert!(dropped.is_empty());
}

#[test]
fn supported_options_pass_through_including_nulls() {
    let mapped = map_openai_params(
        &params(&[
            ("seed", number(7)),
            ("image_url", text("https://example.test/input.png")),
            ("aspect_ratio", text("16:9")),
            ("prompt_upsampling", Json::Bool(true)),
            ("raw", Json::Null),
        ]),
        IndexMap::new(),
        "flux-dev",
        false,
    )
    .unwrap();

    assert_eq!(mapped.get("seed"), Some(&number(7)));
    assert_eq!(
        mapped.get("image_url"),
        Some(&text("https://example.test/input.png"))
    );
    assert_eq!(mapped.get("aspect_ratio"), Some(&text("16:9")));
    assert_eq!(mapped.get("prompt_upsampling"), Some(&Json::Bool(true)));
    assert_eq!(mapped.get("raw"), Some(&Json::Null));
}

#[test]
fn the_request_body_projects_only_the_listed_fields() {
    let optional = params(&[
        ("width", number(1024)),
        ("height", number(1024)),
        ("seed", Json::Null),
        ("image_url", text("https://example.test/a.png")),
        ("size", text("1024x1024")),
        ("quality", text("hd")),
        ("output_format", text("jpeg")),
    ]);
    let body = request_body("a cat", &optional);

    assert_eq!(
        body,
        params(&[
            ("prompt", text("a cat")),
            ("width", number(1024)),
            ("height", number(1024)),
            ("image_url", text("https://example.test/a.png")),
            ("output_format", text("jpeg")),
        ])
    );
}

#[test]
fn the_request_body_supplies_png_when_no_output_format_is_set() {
    let body = request_body("a cat", &IndexMap::new());

    assert_eq!(body.get("output_format"), Some(&text("png")));
    assert_eq!(body.len(), 2);
}

#[rstest]
#[case::caller_base(Some("https://caller.test//"), &[("BFL_API_BASE", "https://env.test/")], "https://caller.test/v1/flux-pro-1.1")]
#[case::env_base(None, &[("BFL_API_BASE", "https://env.test/")], "https://env.test/v1/flux-pro-1.1")]
#[case::default_base(None, &[], "https://api.bfl.ai/v1/flux-pro-1.1")]
#[case::empty_caller_base(Some(""), &[("BFL_API_BASE", "https://env.test")], "https://env.test/v1/flux-pro-1.1")]
#[case::empty_env_base(None, &[("BFL_API_BASE", "")], "https://api.bfl.ai/v1/flux-pro-1.1")]
fn resolves_the_complete_url(
    #[case] api_base: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    let (_, configured_base) = settings(environment);

    assert_eq!(
        complete_url(api_base, "flux-pro-1.1", &configured_base).unwrap(),
        expected
    );
}

#[test]
fn the_complete_url_rejects_an_unknown_model() {
    let (_, configured_base) = settings(&[]);

    assert!(matches!(
        complete_url(None, "flux-unknown", &configured_base),
        Err(BflError::UnknownModel { .. })
    ));
}

#[test]
fn an_explicit_api_key_wins_over_both_environment_names() {
    let (configured_key, _) = settings(&[
        ("BFL_API_KEY", "env-bfl"),
        ("BLACK_FOREST_LABS_API_KEY", "env-legacy"),
    ]);
    let headers = validate_environment(
        HeaderMap::new(),
        Some(&secret("caller")),
        configured_key.as_ref(),
    )
    .unwrap();

    assert_eq!(headers.get("x-key").unwrap(), "caller");
}

#[test]
fn bfl_api_key_wins_over_black_forest_labs_api_key() {
    let (configured_key, _) = settings(&[
        ("BFL_API_KEY", "env-bfl"),
        ("BLACK_FOREST_LABS_API_KEY", "env-legacy"),
    ]);
    let headers = validate_environment(HeaderMap::new(), None, configured_key.as_ref()).unwrap();

    assert_eq!(headers.get("x-key").unwrap(), "env-bfl");
}

#[test]
fn black_forest_labs_api_key_is_the_fallback() {
    let (configured_key, _) = settings(&[("BLACK_FOREST_LABS_API_KEY", "env-legacy")]);
    let headers = validate_environment(HeaderMap::new(), None, configured_key.as_ref()).unwrap();

    assert_eq!(headers.get("x-key").unwrap(), "env-legacy");
}

#[rstest]
#[case::empty_caller_key("", &[("BFL_API_KEY", "env-bfl")], "env-bfl")]
#[case::empty_bfl_key("", &[("BFL_API_KEY", ""), ("BLACK_FOREST_LABS_API_KEY", "env-legacy")], "env-legacy")]
fn empty_credentials_fall_through(
    #[case] api_key: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    let (configured_key, _) = settings(environment);
    let key = secret(api_key);
    let headers =
        validate_environment(HeaderMap::new(), Some(&key), configured_key.as_ref()).unwrap();

    assert_eq!(headers.get("x-key").unwrap(), expected);
}

#[test]
fn missing_credentials_raise_the_source_401() {
    let (configured_key, _) = settings(&[]);
    let error = validate_environment(HeaderMap::new(), None, configured_key.as_ref()).unwrap_err();
    let empty = validate_environment(HeaderMap::new(), Some(&secret("")), None).unwrap_err();

    assert!(matches!(&error, BflError::MissingCredentials));
    assert!(matches!(&empty, BflError::MissingCredentials));
    assert_eq!(error.status(), Some(401));
}

#[test]
fn the_three_headers_overwrite_caller_values() {
    let mut headers = HeaderMap::new();
    headers.insert("x-key", "stale".parse().unwrap());
    headers.insert("content-type", "text/plain".parse().unwrap());
    headers.insert("accept", "text/plain".parse().unwrap());
    let prepared = validate_environment(headers, Some(&secret("fresh")), None).unwrap();

    assert_eq!(prepared.get("x-key").unwrap(), "fresh");
    assert_eq!(prepared.get("content-type").unwrap(), "application/json");
    assert_eq!(prepared.get("accept").unwrap(), "application/json");
    assert!(prepared.get("x-key").unwrap().is_sensitive());
}

#[test]
fn the_api_key_never_appears_in_debug_text() {
    let key = secret("super-secret");
    let headers = validate_environment(HeaderMap::new(), Some(&key), None).unwrap();
    let rendered = format!("{headers:?}");

    assert!(!rendered.contains("super-secret"));
    assert_eq!(key.expose_secret(), "super-secret");
}

#[test]
fn a_reply_decodes_status_url_and_error_presence() {
    let decoded = reply(
        r#"{"status":"Ready","polling_url":"https://api.bfl.ai/v1/get_result?id=1","errors":[]}"#,
    );

    assert_eq!(decoded.status.as_deref(), Some("Ready"));
    assert_eq!(
        decoded.polling_url.as_deref(),
        Some("https://api.bfl.ai/v1/get_result?id=1")
    );
    assert!(decoded.errors.is_some());
}

#[test]
fn a_null_errors_key_is_still_present() {
    let decoded = reply(r#"{"status":"Pending","errors":null}"#);

    assert_eq!(decoded.status.as_deref(), Some("Pending"));
    assert!(decoded.errors.is_some());
}

#[test]
fn an_absent_errors_key_is_not_present() {
    let decoded = reply(r#"{"status":"Pending"}"#);

    assert!(decoded.errors.is_none());
    assert!(decoded.polling_url.is_none());
}

#[test]
fn a_sample_result_projects_one_url() {
    let decoded = reply(r#"{"status":"Ready","result":{"sample":"https://delivery.test/a.png"}}"#);

    assert_eq!(
        urls(&decoded, Vec::new()),
        Vec::from([Some("https://delivery.test/a.png".to_owned())])
    );
}

#[test]
fn a_list_result_projects_strings_and_url_objects_only() {
    let decoded = reply(
        r#"{"result":["https://delivery.test/a.png",{"url":"https://delivery.test/b.png"},{"width":1},7]}"#,
    );

    assert_eq!(
        urls(&decoded, Vec::new()),
        Vec::from([
            Some("https://delivery.test/a.png".to_owned()),
            Some("https://delivery.test/b.png".to_owned())
        ])
    );
}

#[rstest]
#[case::string_result(r#"{"result":"https://delivery.test/a.png"}"#)]
#[case::number_result(r#"{"result":7}"#)]
#[case::null_result(r#"{"result":null}"#)]
#[case::object_result(r#"{"result":{"width":1}}"#)]
#[case::absent_result(r#"{"status":"Pending"}"#)]
fn results_without_images_raise_the_source_500(#[case] body: &str) {
    let decoded = reply(body);
    let error = apply_image_response(&decoded, Vec::new(), 1).unwrap_err();

    assert!(matches!(&error, BflError::NoImageUrl));
    assert_eq!(error.status(), Some(500));
}

#[test]
fn malformed_replies_keep_their_decode_error() {
    let error = decode_bfl_reply(b"{").unwrap_err();
    let trailing = decode_bfl_reply(br#"{"status":"Ready"} {}"#).unwrap_err();
    let wrong_status = decode_bfl_reply(br#"{"status":7}"#).unwrap_err();

    assert!(matches!(&error, BflError::ReplyDecode { .. }));
    assert!(matches!(&trailing, BflError::ReplyTrailing { .. }));
    assert!(matches!(&wrong_status, BflError::ReplyDecode { .. }));
    assert_eq!(error.status(), None);
    assert_eq!(trailing.status(), None);
}

#[test]
fn a_reply_past_the_size_limit_is_refused() {
    let oversized = vec![b' '; MAX_REPLY_BYTES.saturating_add(1)];
    let error = decode_bfl_reply(&oversized).unwrap_err();

    assert!(matches!(&error, BflError::ReplyTooLarge { limit } if *limit == MAX_REPLY_BYTES));
}

#[test]
fn a_reply_at_the_size_limit_is_accepted() {
    let padding = "a".repeat(MAX_REPLY_BYTES - 13);
    let body = format!("{{\"status\":\"{padding}\"}}");
    let decoded = decode_bfl_reply(body.as_bytes()).unwrap();

    assert_eq!(body.len(), MAX_REPLY_BYTES);
    assert_eq!(decoded.status.as_deref().map(str::len), Some(padding.len()));
}

#[rstest]
#[case::registered_domain("https://bfl.ai/v1/get_result")]
#[case::api_host("https://api.bfl.ai/v1/get_result?id=1")]
#[case::gateway("https://gateway.bfl.ai/v1/get_result?id=1")]
#[case::upper_case("HTTPS://GATEWAY.BFL.AI/v1/get_result")]
#[case::port("https://gateway.bfl.ai:8443/v1/get_result")]
#[case::userinfo("https://user:pass@gateway.bfl.ai/v1/get_result")]
fn accepts_https_bfl_hosts(#[case] polling_url: &str) {
    let parsed = check_bfl_poll_url(polling_url).unwrap();

    assert_eq!(parsed.scheme(), "https");
}

#[rstest]
#[case::http("http://api.bfl.ai/v1/get_result", BflError::PollingScheme)]
#[case::suffix_confusion("https://bfl.ai.example.test/v1/get_result", BflError::PollingHost)]
#[case::lookalike("https://evilbfl.ai/v1/get_result", BflError::PollingHost)]
#[case::other_domain("https://example.test/v1/get_result", BflError::PollingHost)]
#[case::userinfo_host("https://bfl.ai@evil.test/v1/get_result", BflError::PollingHost)]
#[case::relative("v1/get_result", BflError::PollingUrl { source: url::ParseError::RelativeUrlWithoutBase })]
fn rejects_untrusted_polling_urls(#[case] polling_url: &str, #[case] expected: BflError) {
    let error = check_bfl_poll_url(polling_url).unwrap_err();

    assert_eq!(
        std::mem::discriminant(&error),
        std::mem::discriminant(&expected)
    );
    assert_eq!(error.status(), Some(502));
}

#[test]
fn a_sample_is_appended_to_existing_image_data() {
    let decoded = reply(r#"{"result":{"sample":"https://delivery.test/a.png"}}"#);
    let response =
        apply_image_response(&decoded, Vec::from([existing_image()]), 1_700_000_000).unwrap();

    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.data.len(), 2);
    assert_eq!(
        response.data[0].url.as_deref(),
        Some("https://delivery.test/old.png")
    );
    assert_eq!(
        response.data[1].url.as_deref(),
        Some("https://delivery.test/a.png")
    );
    assert!(response.data[1].b64_json.is_none());
    assert!(response.data[1].revised_prompt.is_none());
    assert!(response.data[1].provider_specific_fields.is_none());
}

#[test]
fn an_existing_image_lets_an_empty_result_succeed() {
    let decoded = reply(r#"{"result":{}}"#);
    let response = apply_image_response(&decoded, Vec::from([existing_image()]), 9).unwrap();

    assert_eq!(response.created, 9);
    assert_eq!(response.data.len(), 1);
    assert_eq!(
        response.data[0].url.as_deref(),
        Some("https://delivery.test/old.png")
    );
}

#[rstest]
fn replays_the_bfl_boundary_corpus(#[files("fuzz/corpus/bfl_boundary/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_bfl_reply(&bytes));
    if let Ok(candidate) = std::str::from_utf8(&bytes) {
        drop(check_bfl_poll_url(candidate));
    }
}
