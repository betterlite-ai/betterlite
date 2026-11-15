#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageFile, ImageHeaders, ImageHttpResponse, ImageParams,
};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::error::StabilityEditError;
use super::reply::{COST_HEADER, ImageEditReply, ModelInfoLookup, transform_image_edit_response};
use super::request::{
    EDIT_FILE_FIELDS, EDIT_TEXT_FIELDS, StabilityImageEditRequest, transform_image_edit_request,
};
use super::types::{
    DEFAULT_API_BASE, DEFAULT_EDIT_ENDPOINT, EditOptions, EditValue, model_endpoint,
};
use super::{
    StabilityImageEditConfig, StabilityImageEditSettings, get_stability_image_edit_config,
};
use crate::api::stability::generation::reply::StabilityReplyError;

const NOW: i64 = 1_700_000_000;
const INPAINT: &str = "stability/inpaint";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(entries: &[(&str, &str)]) -> StabilityImageEditConfig {
    get_stability_image_edit_config(INPAINT, &lookup(entries))
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn options(entries: &[(&str, Json)]) -> EditOptions {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), EditValue::Json(value.clone())))
        .collect()
}

fn with_file(mut options: EditOptions, name: &str, files: Vec<ImageFile>) -> EditOptions {
    options.insert(name.to_owned(), EditValue::Files(files));
    options
}

fn file(content: &[u8]) -> ImageFile {
    ImageFile {
        filename: None,
        content: content.to_vec(),
        content_type: None,
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn integer(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture is JSON")
}

fn upstream(status: u16, body: &str) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: status,
        headers: ImageHeaders::from([("x-request-id".to_owned(), "req".to_owned())]),
        body: body.as_bytes().to_vec(),
    }
}

fn image(value: &str) -> ImageData {
    ImageData {
        b64_json: Some(value.to_owned()),
        url: None,
        revised_prompt: None,
    }
}

struct Catalog(JsonObject);

#[derive(Debug, thiserror::Error)]
#[error("the model is not mapped")]
struct Unmapped;

impl ModelInfoLookup for Catalog {
    type Error = Unmapped;

    fn model_info(&self, _model: &str) -> Result<JsonObject, Unmapped> {
        Ok(self.0.clone())
    }
}

struct Missing;

impl ModelInfoLookup for Missing {
    type Error = Unmapped;

    fn model_info(&self, _model: &str) -> Result<JsonObject, Unmapped> {
        Err(Unmapped)
    }
}

fn catalog(source: &str) -> Catalog {
    Catalog(serde_json::from_str(source).expect("the catalog entry is a price record"))
}

fn response<Lookup>(
    lookup: &Lookup,
    body: &str,
) -> Result<ImageEditReply, StabilityEditError<Lookup::Error>>
where
    Lookup: ModelInfoLookup,
{
    transform_image_edit_response(INPAINT, &upstream(200, body), NOW, lookup)
}

fn header(reply: &ImageEditReply) -> Option<&Json> {
    reply.additional_headers.get(COST_HEADER)
}

fn part_names(request: &StabilityImageEditRequest) -> Vec<&str> {
    request
        .files
        .iter()
        .map(|(name, _file)| name.as_str())
        .collect()
}

#[test]
fn the_supported_openai_params_are_n_size_response_format_and_mask() {
    assert_eq!(
        StabilityImageEditConfig::SUPPORTED_OPENAI_PARAMS,
        ["n", "size", "response_format", "mask"]
    );
}

#[test]
fn the_adapter_reports_multipart_form_data() {
    const {
        assert!(StabilityImageEditConfig::MULTIPART_FORM_DATA);
    }
}

#[rstest]
#[case::unknown_model("stability/not-a-model", DEFAULT_EDIT_ENDPOINT)]
#[case::inpaint("stability/inpaint", "/v2beta/stable-image/edit/inpaint")]
#[case::outpaint("stability/outpaint", "/v2beta/stable-image/edit/outpaint")]
#[case::erase("stability/erase", "/v2beta/stable-image/edit/erase")]
#[case::search_and_replace(
    "stability/search-and-replace",
    "/v2beta/stable-image/edit/search-and-replace"
)]
#[case::search_and_recolor(
    "stability/search-and-recolor",
    "/v2beta/stable-image/edit/search-and-recolor"
)]
#[case::remove_background(
    "stability/remove-background",
    "/v2beta/stable-image/edit/remove-background"
)]
#[case::replace_background_and_relight(
    "stability/replace-background-and-relight",
    "/v2beta/stable-image/edit/replace-background-and-relight"
)]
#[case::upscale_fast("stability/fast", "/v2beta/stable-image/upscale/fast")]
#[case::upscale_conservative("stability/conservative", "/v2beta/stable-image/upscale/conservative")]
#[case::upscale_creative("stability/creative", "/v2beta/stable-image/upscale/creative")]
#[case::sketch("stability/sketch", "/v2beta/stable-image/control/sketch")]
#[case::structure("stability/structure", "/v2beta/stable-image/control/structure")]
#[case::style("stability/style", "/v2beta/stable-image/control/style")]
#[case::style_transfer_is_shadowed(
    "stability/style-transfer",
    "/v2beta/stable-image/control/style"
)]
#[case::uppercase("STABILITY/Style-Transfer", "/v2beta/stable-image/control/style")]
#[case::unprefixed("Style-Transfer", "/v2beta/stable-image/control/style")]
#[case::embedded_prefix("models/stability/style", "/v2beta/stable-image/control/style")]
#[case::first_key_wins("erase-inpaint", "/v2beta/stable-image/edit/inpaint")]
#[case::empty("", DEFAULT_EDIT_ENDPOINT)]
fn the_endpoint_is_the_first_table_key_inside_the_lowercased_model(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(model_endpoint(model), expected);
}

#[rstest]
#[case::default_base(&[], None, "https://api.stability.ai")]
#[case::environment_base(
    &[("STABILITY_API_BASE", "https://env.test/v1")],
    None,
    "https://env.test/v1"
)]
#[case::explicit_base_wins(
    &[("STABILITY_API_BASE", "https://env.test/v1")],
    Some("https://caller.test/base"),
    "https://caller.test/base"
)]
#[case::empty_explicit_base_falls_through(
    &[("STABILITY_API_BASE", "https://env.test/v1")],
    Some(""),
    "https://env.test/v1"
)]
#[case::trailing_slashes_are_trimmed(&[], Some("https://caller.test///"), "https://caller.test")]
fn the_complete_url_takes_a_caller_base_before_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let config = config(environment);

    assert_eq!(
        config.get_complete_url(api_base, &ImageParams::new(), INPAINT),
        format!("{expected}{DEFAULT_EDIT_ENDPOINT}")
    );
}

#[rstest]
#[case::litellm_base(&[], Some("https://litellm.test"), "https://litellm.test")]
#[case::empty_litellm_base(&[], Some(""), DEFAULT_API_BASE)]
#[case::environment_before_litellm_base(
    &[("STABILITY_API_BASE", "https://env.test/v1")],
    Some("https://litellm.test"),
    "https://env.test/v1"
)]
#[case::empty_environment_counts_as_unset(
    &[("STABILITY_API_BASE", "")],
    Some("https://litellm.test"),
    "https://litellm.test"
)]
#[case::environment_slashes_are_trimmed(
    &[("STABILITY_API_BASE", "https://env.test/v1//")],
    None,
    "https://env.test/v1"
)]
fn litellm_params_base_follows_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] litellm_base: Option<&str>,
    #[case] expected: &str,
) {
    let config = config(environment);
    let litellm_params = match litellm_base {
        Some(base) => params(&[("api_base", text(base))]),
        None => ImageParams::new(),
    };

    assert_eq!(
        config.get_complete_url(None, &litellm_params, INPAINT),
        format!("{expected}{DEFAULT_EDIT_ENDPOINT}")
    );
}

#[test]
fn a_non_text_litellm_params_base_is_not_a_base() {
    let config = config(&[]);

    assert_eq!(
        config.get_complete_url(None, &params(&[("api_base", Json::Null)]), INPAINT),
        format!("{DEFAULT_API_BASE}{DEFAULT_EDIT_ENDPOINT}")
    );
}

#[test]
fn validate_environment_fails_without_a_key() {
    let error = config(&[])
        .validate_environment(ImageHeaders::new(), None)
        .expect_err("a missing key must fail");

    assert!(matches!(error, StabilityEditError::MissingApiKey));
}

#[test]
fn validate_environment_overwrites_the_auth_and_accept_headers() {
    let config = config(&[("STABILITY_API_KEY", "server-key")]);
    let mut headers = ImageHeaders::new();
    headers.insert("Authorization".to_owned(), "Bearer stale".to_owned());
    headers.insert("x-extra".to_owned(), "kept".to_owned());

    let validated = config
        .validate_environment(headers, None)
        .expect("the environment key is set");

    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer server-key")
    );
    assert_eq!(
        validated.get("Accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("x-extra").map(String::as_str), Some("kept"));
}

#[test]
fn an_explicit_key_beats_the_environment_key() {
    let config = config(&[("STABILITY_API_KEY", "server-key")]);
    let explicit = SecretString::from("caller-key");

    let headers = config
        .validate_environment(ImageHeaders::new(), Some(&explicit))
        .expect("the explicit key is set");

    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer caller-key")
    );
}

#[test]
fn an_empty_explicit_key_falls_through_to_the_environment_key() {
    let config = config(&[("STABILITY_API_KEY", "server-key")]);
    let explicit = SecretString::from("");

    let headers = config
        .validate_environment(ImageHeaders::new(), Some(&explicit))
        .expect("the environment key is set");

    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer server-key")
    );
}

#[test]
fn debug_never_shows_the_key() {
    let settings = StabilityImageEditSettings::from_environment(&lookup(&[
        ("STABILITY_API_KEY", "server-key"),
        ("STABILITY_API_BASE", "https://env.test"),
    ]));

    let rendered = format!("{settings:?}");

    assert!(!rendered.contains("server-key"));
}

#[test]
fn the_factory_loads_the_environment_base() {
    let config = config(&[
        ("STABILITY_API_KEY", "server-key"),
        ("STABILITY_API_BASE", "https://env.test"),
    ]);

    assert_eq!(
        config.settings.api_base.as_deref(),
        Some("https://env.test")
    );
    assert_eq!(
        config.get_complete_url(None, &ImageParams::new(), INPAINT),
        "https://env.test/v2beta/stable-image/edit/inpaint"
    );
}

#[test]
fn an_unset_environment_base_stays_absent() {
    let config = config(&[("STABILITY_API_KEY", "server-key")]);

    assert_eq!(config.settings.api_base, None);
}

#[rstest]
#[case::square("1024x1024", "1:1")]
#[case::landscape("1792x1024", "16:9")]
#[case::portrait("1024x1792", "9:16")]
#[case::small_square("512x512", "1:1")]
#[case::tiny_square("256x256", "1:1")]
fn a_known_size_maps_to_its_aspect_ratio(#[case] size: &str, #[case] expected: &str) {
    let input = options(&[("size", text(size))]);

    let mapped = StabilityImageEditConfig::map_openai_params(&input, INPAINT, false)
        .expect("size is supported");

    assert_eq!(
        mapped.get("aspect_ratio"),
        Some(&EditValue::Json(text(expected)))
    );
    assert!(!mapped.contains_key("size"));
}

#[rstest]
#[case::unknown_size(Json::Text(String::from("999x999")))]
#[case::wrong_typed_size(Json::Number(Number::from(1024)))]
#[case::null_size(Json::Null)]
fn a_size_outside_the_table_is_dropped(#[case] size: Json) {
    let input = options(&[("size", size)]);

    let mapped = StabilityImageEditConfig::map_openai_params(&input, INPAINT, false)
        .expect("size is supported");

    assert!(mapped.is_empty());
}

#[test]
fn a_known_size_overwrites_a_supplied_aspect_ratio() {
    let input = options(&[("aspect_ratio", text("3:2")), ("size", text("1024x1024"))]);

    let mapped = StabilityImageEditConfig::map_openai_params(&input, INPAINT, false)
        .expect("size is supported");

    assert_eq!(
        mapped.get("aspect_ratio"),
        Some(&EditValue::Json(text("1:1")))
    );
}

#[test]
fn n_and_response_format_are_stashed_internal() {
    let input = options(&[("n", integer(2)), ("response_format", text("b64_json"))]);

    let mapped = StabilityImageEditConfig::map_openai_params(&input, INPAINT, false)
        .expect("both names are supported");

    assert_eq!(mapped.get("_n"), Some(&EditValue::Json(integer(2))));
    assert_eq!(
        mapped.get("_response_format"),
        Some(&EditValue::Json(text("b64_json")))
    );
    assert!(!mapped.contains_key("n"));
    assert!(!mapped.contains_key("response_format"));
}

#[test]
fn a_supported_mask_survives_mapping() {
    let input = options(&[("mask", text("mask-id"))]);

    let mapped = StabilityImageEditConfig::map_openai_params(&input, INPAINT, false)
        .expect("mask is supported");

    assert_eq!(mapped.get("mask"), Some(&EditValue::Json(text("mask-id"))));
}

#[test]
fn a_file_valued_option_survives_mapping() {
    let input = with_file(options(&[]), "init_image", vec![file(b"init")]);

    let mapped = StabilityImageEditConfig::map_openai_params(&input, INPAINT, true)
        .expect("drop_params keeps init_image");

    assert_eq!(
        mapped.get("init_image"),
        Some(&EditValue::Files(vec![file(b"init")]))
    );
}

#[test]
fn mapping_leaves_the_input_alone() {
    let input = options(&[("size", text("1024x1024")), ("n", integer(3))]);

    let mapped = StabilityImageEditConfig::map_openai_params(&input, INPAINT, false)
        .expect("both names are supported");

    assert_eq!(input.get("size"), Some(&EditValue::Json(text("1024x1024"))));
    assert_eq!(input.get("n"), Some(&EditValue::Json(integer(3))));
    assert!(!mapped.contains_key("size"));
}

#[test]
fn an_unsupported_parameter_raises_without_drop_params() {
    let input = options(&[("quality", text("high"))]);

    let error = StabilityImageEditConfig::map_openai_params(&input, INPAINT, false)
        .expect_err("quality is unsupported");

    assert!(matches!(
        error,
        StabilityEditError::UnsupportedParameter { parameter, model }
            if parameter == "quality" && model == INPAINT
    ));
}

#[test]
fn drop_params_keeps_the_unsupported_parameters_in_the_copy() {
    let input = options(&[
        ("quality", text("high")),
        ("negative_prompt", text("blur")),
        ("size", text("1024x1024")),
    ]);

    let mapped = StabilityImageEditConfig::map_openai_params(&input, INPAINT, true)
        .expect("drop_params keeps quality and negative_prompt");

    assert_eq!(mapped.get("quality"), Some(&EditValue::Json(text("high"))));
    assert_eq!(
        mapped.get("negative_prompt"),
        Some(&EditValue::Json(text("blur")))
    );
    assert_eq!(
        mapped.get("aspect_ratio"),
        Some(&EditValue::Json(text("1:1")))
    );
    assert!(!mapped.contains_key("size"));
}

#[test]
fn an_internal_name_in_the_input_is_unsupported() {
    let input = options(&[("_n", integer(2))]);

    let error = StabilityImageEditConfig::map_openai_params(&input, INPAINT, false)
        .expect_err("_n is not a supported OpenAI parameter");

    assert!(matches!(
        error,
        StabilityEditError::UnsupportedParameter { .. }
    ));
}

#[test]
fn an_empty_map_stays_empty() {
    let mapped = StabilityImageEditConfig::map_openai_params(&options(&[]), INPAINT, false)
        .expect("there is nothing to map");

    assert!(mapped.is_empty());
}

#[test]
fn the_request_starts_with_png_output_and_the_prompt() {
    let request = transform_image_edit_request(Some("a cat"), None, &options(&[]))
        .expect("the request is built");

    let serialized = serde_json::to_string(&request.data).expect("the data serializes");
    assert_eq!(
        json(&serialized),
        json(r#"{"output_format": "png", "prompt": "a cat"}"#)
    );
    assert!(request.files.is_empty());
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn a_missing_or_empty_prompt_is_omitted(#[case] prompt: Option<&str>) {
    let request =
        transform_image_edit_request(prompt, None, &options(&[])).expect("the request is built");

    assert_eq!(request.data.get("prompt"), None);
}

#[test]
fn the_first_image_list_member_becomes_the_image_part() {
    let images = [file(b"first"), file(b"second")];

    let request = transform_image_edit_request(Some("a cat"), Some(&images), &options(&[]))
        .expect("the request is built");

    assert_eq!(request.files, vec![("image".to_owned(), file(b"first"))]);
}

#[test]
fn an_empty_image_list_fails() {
    let error = transform_image_edit_request(Some("a cat"), Some(&[]), &options(&[]))
        .expect_err("an edit needs an image");

    assert!(matches!(
        error,
        StabilityEditError::MissingFile { field } if field == "image"
    ));
}

#[test]
fn the_image_and_file_options_keep_the_litellm_part_order() {
    let images = [file(b"image")];
    let input = with_file(
        with_file(
            with_file(
                options(&[("seed", integer(7))]),
                "style_image",
                vec![file(b"style")],
            ),
            "mask",
            vec![file(b"mask")],
        ),
        "init_image",
        vec![file(b"init")],
    );

    let request = transform_image_edit_request(Some("a cat"), Some(&images), &input)
        .expect("the request is built");

    assert_eq!(
        part_names(&request),
        ["image", "style_image", "mask", "init_image"]
    );
}

#[rstest]
#[case::mask("mask")]
#[case::init_image("init_image")]
#[case::style_image("style_image")]
fn an_empty_file_option_list_fails(#[case] name: &str) {
    let input = with_file(options(&[]), name, Vec::new());

    let error = transform_image_edit_request(Some("a cat"), None, &input)
        .expect_err("an empty file list has no first member");

    assert!(matches!(
        error,
        StabilityEditError::MissingFile { field } if field == name
    ));
}

#[rstest]
#[case::mask("mask", "mask-id")]
#[case::init_image("init_image", "init-id")]
#[case::style_image("style_image", "style-id")]
fn a_text_file_option_is_refused(#[case] name: &str, #[case] value: &str) {
    let input = options(&[(name, text(value))]);

    let error = transform_image_edit_request(Some("a cat"), None, &input)
        .expect_err("a file part is not a form field");

    assert!(matches!(
        error,
        StabilityEditError::RequestField { field } if field == name
    ));
}

#[test]
fn a_null_file_option_is_omitted() {
    let input = options(&[("mask", Json::Null)]);

    let request = transform_image_edit_request(Some("a cat"), None, &input)
        .expect("a null option is skipped");

    assert!(request.files.is_empty());
}

#[test]
fn internal_and_unknown_options_never_reach_the_body() {
    let input = options(&[
        ("_n", integer(2)),
        ("_response_format", text("b64_json")),
        ("quality", text("high")),
        ("seed", integer(7)),
    ]);

    let request =
        transform_image_edit_request(Some("a cat"), None, &input).expect("the request is built");

    let serialized = serde_json::to_string(&request.data).expect("the data serializes");
    assert_eq!(
        json(&serialized),
        json(r#"{"output_format": "png", "prompt": "a cat", "seed": 7}"#)
    );
}

#[test]
fn a_supplied_output_format_never_replaces_png() {
    let input = options(&[("output_format", text("jpeg"))]);

    let request =
        transform_image_edit_request(Some("a cat"), None, &input).expect("the request is built");

    assert_eq!(request.data.get("output_format"), Some(&text("png")));
}

#[rstest]
#[case::negative_prompt("negative_prompt", text("blur"))]
#[case::aspect_ratio("aspect_ratio", text("16:9"))]
#[case::seed("seed", integer(7))]
#[case::mode("mode", text("image-to-image"))]
#[case::strength("strength", integer(1))]
#[case::style_preset("style_preset", text("anime"))]
#[case::left("left", integer(1))]
#[case::bottom("bottom", integer(2))]
#[case::right("right", integer(3))]
#[case::top("top", integer(4))]
#[case::creativity("creativity", integer(1))]
#[case::search_prompt("search_prompt", text("the cat"))]
#[case::grow_mask("grow_mask", integer(5))]
#[case::select_prompt("select_prompt", text("the dog"))]
#[case::control_strength("control_strength", integer(1))]
#[case::composition_fidelity("composition_fidelity", integer(1))]
#[case::change_strength("change_strength", integer(1))]
fn every_allowlisted_text_field_reaches_the_body(#[case] name: &str, #[case] value: Json) {
    let input = options(&[(name, value.clone())]);

    let request = transform_image_edit_request(Some("a cat"), None, &input)
        .expect("the field is allowlisted");

    assert_eq!(request.data.get(name), Some(&value));
    assert_eq!(request.data.len(), 3);
}

#[test]
fn a_fractional_strength_keeps_its_number() {
    let strength = Json::Number(Number::from_f64(0.5).expect("0.5 is a JSON number"));
    let input = options(&[("strength", strength.clone())]);

    let request =
        transform_image_edit_request(None, None, &input).expect("strength is allowlisted");

    assert_eq!(request.data.get("strength"), Some(&strength));
}

#[rstest]
#[case::seed(text("seven"), "seed")]
#[case::left(Json::Bool(true), "left")]
#[case::strength(text("half"), "strength")]
#[case::grow_mask(text("5"), "grow_mask")]
#[case::search_prompt(integer(5), "search_prompt")]
#[case::negative_prompt(integer(5), "negative_prompt")]
fn a_wrong_typed_text_field_names_its_field(#[case] value: Json, #[case] field: &str) {
    let input = options(&[(field, value)]);

    let error = transform_image_edit_request(Some("a cat"), None, &input)
        .expect_err("the value has the wrong type");

    assert!(matches!(
        error,
        StabilityEditError::RequestField { field: named } if named == field
    ));
}

#[test]
fn a_file_value_under_a_text_field_is_refused() {
    let input = with_file(options(&[]), "seed", vec![file(b"seed")]);

    let error = transform_image_edit_request(Some("a cat"), None, &input)
        .expect_err("seed is a form field, not a file part");

    assert!(matches!(
        error,
        StabilityEditError::RequestField { field } if field == "seed"
    ));
}

#[test]
fn the_text_and_file_tables_hold_the_litellm_names() {
    assert_eq!(EDIT_TEXT_FIELDS.len(), 17);
    assert_eq!(EDIT_FILE_FIELDS, ["mask", "init_image", "style_image"]);
}

#[test]
fn the_catalog_cost_becomes_a_float_header() {
    let reply = response(
        &catalog(r#"{"output_cost_per_image": 0.04, "litellm_provider": "stability"}"#),
        r#"{"image": "abc", "finish_reason": "SUCCESS"}"#,
    )
    .expect("the reply holds an image");

    assert_eq!(reply.response.created, NOW);
    assert_eq!(reply.response.data, Some(vec![image("abc")]));
    assert_eq!(
        header(&reply),
        Some(&Json::Number(
            Number::from_f64(0.04).expect("0.04 is a JSON number")
        ))
    );
}

#[test]
fn a_missing_catalog_cost_defaults_to_zero() {
    let reply = response(
        &catalog(r#"{"litellm_provider": "stability"}"#),
        r#"{"image": "abc"}"#,
    )
    .expect("the reply holds an image");

    assert_eq!(
        header(&reply),
        Some(&Json::Number(
            Number::from_f64(0.0).expect("0.0 is a JSON number")
        ))
    );
}

#[test]
fn a_null_catalog_cost_suppresses_the_header() {
    let reply = response(
        &catalog(r#"{"output_cost_per_image": null}"#),
        r#"{"image": "abc"}"#,
    )
    .expect("the reply holds an image");

    assert_eq!(header(&reply), None);
}

#[rstest]
#[case::text(r#"{"output_cost_per_image": "0.5"}"#, 0.5)]
#[case::padded_text(r#"{"output_cost_per_image": "  0.5  "}"#, 0.5)]
#[case::exponent(r#"{"output_cost_per_image": 5e-2}"#, 0.05)]
#[case::boolean(r#"{"output_cost_per_image": true}"#, 1.0)]
#[case::zero(r#"{"output_cost_per_image": 0}"#, 0.0)]
fn a_catalog_cost_is_converted_like_python_float(#[case] entry: &str, #[case] expected: f64) {
    let reply = response(&catalog(entry), r#"{"image": "abc"}"#).expect("the cost converts");

    assert_eq!(
        header(&reply),
        Some(&Json::Number(
            Number::from_f64(expected).expect("the expectation is a JSON number")
        ))
    );
}

#[rstest]
#[case::text(r#"{"output_cost_per_image": "abc"}"#)]
#[case::nan(r#"{"output_cost_per_image": "nan"}"#)]
#[case::infinity(r#"{"output_cost_per_image": "inf"}"#)]
#[case::list(r#"{"output_cost_per_image": [1]}"#)]
#[case::object(r#"{"output_cost_per_image": {"amount": 1}}"#)]
fn a_cost_that_is_not_a_finite_number_fails(#[case] entry: &str) {
    let error = response(&catalog(entry), r#"{"image": "abc"}"#)
        .expect_err("the cost is not a finite number");

    assert!(matches!(error, StabilityEditError::CostValue(_)));
}

#[test]
fn an_unmapped_model_fails_the_lookup() {
    let error =
        response(&Missing, r#"{"image": "abc"}"#).expect_err("the model has no price record");

    assert!(matches!(error, StabilityEditError::ModelInfo(Unmapped)));
}

#[test]
fn the_reply_is_decoded_before_the_model_lookup() {
    let error = response(&Missing, r#"{"finish_reason": "CONTENT_FILTERED"}"#)
        .expect_err("the content was filtered");

    assert_eq!(error_status(&error), 400);
    assert!(matches!(
        error,
        StabilityEditError::Reply(StabilityReplyError::ContentFiltered { .. })
    ));
}

fn error_status(error: &StabilityEditError<Unmapped>) -> u16 {
    match error {
        StabilityEditError::Reply(reply) => reply.status(),
        StabilityEditError::MissingApiKey
        | StabilityEditError::UnsupportedParameter { .. }
        | StabilityEditError::RequestField { .. }
        | StabilityEditError::MissingFile { .. }
        | StabilityEditError::CostValue(_)
        | StabilityEditError::ModelInfo(_) => 0,
    }
}

#[test]
fn an_upstream_error_keeps_its_status() {
    let error = transform_image_edit_response(
        INPAINT,
        &upstream(503, r#"{"errors": {"message": "boom"}}"#),
        NOW,
        &catalog(r#"{"output_cost_per_image": 0.04}"#),
    )
    .expect_err("the reply reports an error");

    assert_eq!(error_status(&error), 503);
    assert!(matches!(
        error,
        StabilityEditError::Reply(StabilityReplyError::Upstream { .. })
    ));
}

#[test]
fn an_unparsable_reply_keeps_the_upstream_status() {
    let error = response(&catalog(r#"{"output_cost_per_image": 0.04}"#), "not json")
        .expect_err("the body is not JSON");

    assert_eq!(error_status(&error), 200);
    assert!(matches!(
        error,
        StabilityEditError::Reply(StabilityReplyError::Malformed { .. })
    ));
}

#[rstest]
#[case::empty_image(r#"{"image": "", "finish_reason": "SUCCESS"}"#)]
#[case::missing_image(r#"{"finish_reason": "SUCCESS"}"#)]
fn an_empty_result_still_attaches_the_cost(#[case] body: &str) {
    let reply = response(&catalog(r#"{"output_cost_per_image": 0.05}"#), body)
        .expect("an empty result is not a failure");

    assert_eq!(reply.response.data, Some(Vec::new()));
    assert_eq!(
        header(&reply),
        Some(&Json::Number(
            Number::from_f64(0.05).expect("0.05 is a JSON number")
        ))
    );
}

#[test]
fn a_successful_reply_carries_only_the_cost_header() {
    let reply = response(
        &catalog(r#"{"output_cost_per_image": 0.05}"#),
        r#"{"image": "abc"}"#,
    )
    .expect("the reply holds an image");

    assert_eq!(reply.additional_headers.len(), 1);
}
