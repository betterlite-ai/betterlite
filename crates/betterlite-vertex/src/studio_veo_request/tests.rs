#![cfg(test)]

use std::collections::BTreeMap;
use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_openai::openai_video_identifiers::multipart::ReadSeek;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::vertex_veo_request::image::{B1120Mime, ImageInput, MimeReader};

use super::SUPPORTED_OPENAI_PARAMS;
use super::create::{CreateForm, CreateInput, Parameters, create_request};
use super::environment::{KeySources, validate_environment};
use super::error::StudioVeoError;
use super::params::{MapInput, MappedParams, Reference, map_openai_params};
use super::url::complete_url;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn fraction(value: f64) -> Json {
    Json::Number(serde_json::Number::from_f64(value).expect("a finite fraction"))
}

fn object<const N: usize>(fields: [(&str, Json); N]) -> Object {
    fields
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

struct RefusingMime;

impl MimeReader for RefusingMime {
    fn mime(&self, _reader: &mut dyn ReadSeek) -> &'static str {
        panic!("a dictionary image must not be read");
    }
}

#[test]
fn supported_params_match_litellm() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["model", "prompt", "input_reference", "seconds", "size"]
    );
}

fn map_input() -> MapInput<'static> {
    MapInput {
        input_reference: Reference::Absent,
        image: None,
        size: None,
        seconds: None,
        resolution: None,
        options: Object::new(),
    }
}

#[rstest]
#[case::landscape_720("1280x720", Some("16:9"), Some("720p"))]
#[case::landscape_1080("1920x1080", Some("16:9"), Some("1080p"))]
#[case::portrait_720("720x1280", Some("9:16"), Some("720p"))]
#[case::portrait_1080("1080x1920", Some("9:16"), Some("1080p"))]
#[case::unknown_nonempty("1024x1024", Some("16:9"), None)]
#[case::empty("", None, None)]
fn size_fills_the_aspect_ratio_and_infers_the_resolution(
    #[case] size: &str,
    #[case] ratio: Option<&str>,
    #[case] resolution: Option<&str>,
) {
    let mut input = map_input();
    input.size = Some(size.to_owned());
    let mapped = map_openai_params(input);
    assert_eq!(mapped.options.get("aspectRatio"), ratio.map(text).as_ref());
    assert_eq!(
        mapped.options.get("resolution"),
        resolution.map(text).as_ref()
    );
}

fn mapped_resolution(resolution: Option<Json>) -> Option<Json> {
    let mut input = map_input();
    input.size = Some("1280x720".to_owned());
    input.resolution = resolution;
    map_openai_params(input).options.get("resolution").cloned()
}

#[test]
fn an_explicit_resolution_suppresses_the_size_inference() {
    assert_eq!(mapped_resolution(Some(text("1080p"))), Some(text("1080p")));
}

#[rstest]
#[case::absent(None)]
#[case::null(Some(Json::Null))]
#[case::empty_text(Some(Json::Text(String::new())))]
fn a_missing_resolution_lets_the_size_infer_one(#[case] resolution: Option<Json>) {
    assert_eq!(mapped_resolution(resolution), Some(text("720p")));
}

#[test]
fn an_absent_resolution_lets_a_recognized_size_infer_one() {
    assert_eq!(mapped_resolution(None), Some(text("720p")));
}

#[test]
fn an_empty_text_resolution_lets_the_size_infer_one() {
    assert_eq!(
        mapped_resolution(Some(Json::Text(String::new()))),
        Some(text("720p"))
    );
}

#[test]
fn a_resolution_without_a_recognized_size_is_kept_as_given() {
    let mut input = map_input();
    input.resolution = Some(text(""));
    let mapped = map_openai_params(input);
    assert_eq!(mapped.options.get("resolution"), Some(&text("")));
}

#[test]
fn a_null_resolution_stays_mapped_as_a_null() {
    let mut input = map_input();
    input.resolution = Some(Json::Null);
    let mapped = map_openai_params(input);
    assert_eq!(mapped.options.get("resolution"), Some(&Json::Null));
}

#[rstest]
#[case::plain("8", Some(8))]
#[case::padded(" 08 ", Some(8))]
#[case::signed("+8", Some(8))]
#[case::underscored("1_0", Some(10))]
#[case::zero("0", Some(0))]
#[case::fraction("8.5", None)]
#[case::words("eight", None)]
#[case::empty("", None)]
fn seconds_become_duration_seconds_without_a_default(
    #[case] seconds: &str,
    #[case] expected: Option<i64>,
) {
    let mut input = map_input();
    input.seconds = Some(seconds.to_owned());
    let mapped = map_openai_params(input);
    assert_eq!(
        mapped.options.get("durationSeconds"),
        expected.map(number).as_ref()
    );
}

#[test]
fn no_seconds_maps_no_duration() {
    let mapped = map_openai_params(map_input());
    assert_eq!(mapped.options.get("durationSeconds"), None);
}

#[test]
fn provider_fields_pass_through_unless_the_mapping_set_them() {
    let mut input = map_input();
    input.size = Some("1280x720".to_owned());
    input.seconds = Some("8".to_owned());
    input.options = object([
        ("negativePrompt", text("blurry")),
        ("aspectRatio", text("9:16")),
        ("durationSeconds", number(5)),
        ("model", text("gemini/veo-3")),
        ("prompt", text("a prompt")),
        ("sampleCount", number(3)),
    ]);
    let mapped = map_openai_params(input);
    assert_eq!(mapped.options.get("negativePrompt"), Some(&text("blurry")));
    assert_eq!(mapped.options.get("aspectRatio"), Some(&text("16:9")));
    assert_eq!(mapped.options.get("durationSeconds"), Some(&number(8)));
    assert_eq!(mapped.options.get("model"), Some(&text("gemini/veo-3")));
    assert_eq!(mapped.options.get("prompt"), Some(&text("a prompt")));
    assert_eq!(mapped.options.get("sampleCount"), Some(&number(3)));
}

#[test]
fn a_failed_seconds_conversion_leaves_a_provider_duration() {
    let mut input = map_input();
    input.seconds = Some("eight".to_owned());
    input.options = object([("durationSeconds", number(5))]);
    let mapped = map_openai_params(input);
    assert_eq!(mapped.options.get("durationSeconds"), Some(&number(5)));
}

#[test]
fn input_reference_becomes_the_image_before_the_image_option_does() {
    let mut input = map_input();
    input.input_reference = Reference::Image(ImageInput::Json(Json::Object(object([(
        "gcsUri",
        text("gs://reference"),
    )]))));
    input.image = Some(ImageInput::Json(Json::Object(object([(
        "gcsUri",
        text("gs://image"),
    )]))));
    let mapped = map_openai_params(input);
    let Some(ImageInput::Json(Json::Object(fields))) = mapped.image else {
        panic!("the reference should become the image");
    };
    assert_eq!(fields.get("gcsUri"), Some(&text("gs://reference")));
}

#[test]
fn a_null_input_reference_hides_the_image_option() {
    let mut input = map_input();
    input.input_reference = Reference::Null;
    input.image = Some(ImageInput::Json(Json::Object(object([(
        "gcsUri",
        text("gs://image"),
    )]))));
    let mapped = map_openai_params(input);
    assert!(mapped.image.is_none());
}

#[test]
fn an_absent_input_reference_keeps_the_image_option() {
    let mut input = map_input();
    input.image = Some(ImageInput::Json(text("gs://image")));
    let mapped = map_openai_params(input);
    assert!(matches!(
        mapped.image,
        Some(ImageInput::Json(Json::Text(_)))
    ));
}

fn mapped<'media>(image: Option<ImageInput<'media>>, options: Object) -> MappedParams<'media> {
    MappedParams { image, options }
}

fn create(mapped: MappedParams<'_>, mime: &dyn MimeReader) -> Result<CreateForm, StudioVeoError> {
    create_request(
        CreateInput {
            prompt: "a prompt",
            api_base: "https://mock/veo-3",
            mapped,
        },
        mime,
    )
}

fn image_of(form: &CreateForm) -> Option<&Object> {
    form.body.instances.first()?.image.as_ref()
}

fn parameters_of(form: &CreateForm) -> &Parameters {
    &form.body.parameters
}

fn body_json(form: &CreateForm) -> Json {
    let value = serde_json::to_value(&form.body).expect("the body serializes");
    serde_json::from_value(value).expect("the body is JSON")
}

#[test]
fn create_seeks_a_file_image_to_zero_and_leaves_it_consumed() {
    let data = b"\x89PNG\r\n\x1a\nstudio image".to_vec();
    let mut reader = Cursor::new(data.clone());
    reader.set_position(4);
    let mapped = mapped(Some(ImageInput::File(&mut reader)), Object::new());
    let form = create(mapped, &B1120Mime).expect("the create request is built");
    assert_eq!(
        reader.position(),
        u64::try_from(data.len()).expect("the data fits")
    );
    assert_eq!(form.url, "https://mock/veo-3");
    assert_eq!(
        image_of(&form),
        Some(&object([
            ("bytesBase64Encoded", Json::Text(STANDARD.encode(&data))),
            ("mimeType", text("image/png")),
        ]))
    );
    assert_eq!(parameters_of(&form), &Parameters::default());
}

#[test]
fn create_passes_an_image_dictionary_through_without_reading_it() {
    let image = object([
        ("bytesBase64Encoded", text("abc")),
        ("mimeType", text("image/jpeg")),
    ]);
    let mapped = mapped(
        Some(ImageInput::Json(Json::Object(image.clone()))),
        Object::new(),
    );
    let form = create(mapped, &RefusingMime).expect("the create request is built");
    assert_eq!(image_of(&form), Some(&image));
}

#[rstest]
#[case::string(text("gs://bucket/a.png"))]
#[case::number(number(3))]
#[case::list(Json::List(vec![number(1)]))]
fn create_refuses_an_image_without_a_file_reader(#[case] image: Json) {
    let mapped = mapped(Some(ImageInput::Json(image)), Object::new());
    let error = create(mapped, &RefusingMime).expect_err("only a dict or a file converts");
    assert!(matches!(error, StudioVeoError::ImageNotAFile));
}

#[test]
fn create_drops_a_null_image() {
    let mapped = mapped(Some(ImageInput::Json(Json::Null)), Object::new());
    let form = create(mapped, &RefusingMime).expect("the create request is built");
    assert_eq!(image_of(&form), None);
    assert_eq!(parameters_of(&form), &Parameters::default());
}

#[test]
fn create_always_sends_a_parameters_block() {
    let form = create(mapped(None, Object::new()), &RefusingMime).expect("the body is built");
    assert_eq!(
        body_json(&form),
        Json::Object(object([
            (
                "instances",
                Json::List(vec![Json::Object(object([("prompt", text("a prompt"))]))]),
            ),
            ("parameters", Json::Object(Object::new())),
        ]))
    );
}

#[test]
fn the_parameter_model_keeps_its_eight_declared_fields() {
    let options = object([
        ("aspectRatio", text("16:9")),
        ("durationSeconds", number(8)),
        ("resolution", text("720p")),
        ("negativePrompt", text("blurry")),
        (
            "lastFrame",
            Json::Object(object([("a", Json::Null), ("b", number(1))])),
        ),
        ("referenceImages", Json::List(vec![number(1), Json::Null])),
        ("video", text("files/abc")),
        ("personGeneration", Json::Null),
        ("sampleCount", number(3)),
        (
            "parameters",
            Json::Object(object([("sampleCount", number(2))])),
        ),
    ]);
    let form = create(mapped(None, options), &RefusingMime).expect("the body is built");
    assert_eq!(
        serde_json::to_string(parameters_of(&form)).expect("the parameters serialize"),
        r#"{"aspectRatio":"16:9","durationSeconds":8,"resolution":"720p","negativePrompt":"blurry","lastFrame":{"a":null,"b":1},"referenceImages":[1,null],"video":"files/abc"}"#
    );
}

#[test]
fn the_parameter_model_keeps_an_empty_resolution() {
    let form = create(
        mapped(None, object([("resolution", text(""))])),
        &RefusingMime,
    )
    .expect("the body is built");
    assert_eq!(
        serde_json::to_string(parameters_of(&form)).expect("the parameters serialize"),
        r#"{"resolution":""}"#
    );
}

#[test]
fn the_parameter_model_drops_a_null_resolution() {
    let form = create(
        mapped(None, object([("resolution", Json::Null)])),
        &RefusingMime,
    )
    .expect("the body is built");
    assert_eq!(parameters_of(&form), &Parameters::default());
}

#[rstest]
#[case::aspect_ratio("aspectRatio", number(5))]
#[case::duration_fraction("durationSeconds", fraction(8.5))]
#[case::duration_text("durationSeconds", text("8"))]
#[case::references_object("referenceImages", Json::Object(Object::new()))]
#[case::negative_prompt("negativePrompt", number(5))]
#[case::person_generation("personGeneration", Json::Bool(true))]
fn the_parameter_model_rejects_a_field_of_another_type(#[case] field: &str, #[case] value: Json) {
    let error = create(mapped(None, object([(field, value)])), &RefusingMime)
        .expect_err("the field type does not match");
    let StudioVeoError::Parameters { path, .. } = error else {
        panic!("the error names the field");
    };
    assert_eq!(path, field);
}

fn settings(environment: [Option<&str>; 2], generic: Option<&str>) -> Settings {
    Settings {
        environment_key: environment
            .into_iter()
            .find_map(|key| key.filter(|value| !value.is_empty()))
            .map(SecretString::from),
        generic_api_key: generic.map(SecretString::from),
        api_base: DEFAULT_API_BASE.to_owned(),
    }
}

fn headers_for(
    caller: &BTreeMap<String, String>,
    api_key: Option<&str>,
    params_api_key: Option<&str>,
    settings: &Settings,
) -> Result<BTreeMap<String, SecretString>, StudioVeoError> {
    let api_key = api_key.map(SecretString::from);
    let params_api_key = params_api_key.map(SecretString::from);
    validate_environment(
        caller,
        KeySources {
            api_key: api_key.as_ref(),
            litellm_params_api_key: params_api_key.as_ref(),
        },
        settings,
    )
}

fn key_of(
    caller: &BTreeMap<String, String>,
    api_key: Option<&str>,
    params_api_key: Option<&str>,
    settings: &Settings,
) -> Option<String> {
    let headers = headers_for(caller, api_key, params_api_key, settings).expect("a key is found");
    headers
        .get("x-goog-api-key")
        .map(|value| value.expose_secret().clone())
}

#[test]
fn the_explicit_key_beats_every_other_source() {
    let settings = settings([Some("g"), Some("G")], Some("m"));
    let found = key_of(&BTreeMap::new(), Some("e"), Some("p"), &settings);
    assert_eq!(found.as_deref(), Some("e"));
}

#[rstest]
#[case::empty_explicit("", "p")]
#[case::absent_explicit(None, "p")]
fn params_api_key_follows_the_explicit_key(#[case] api_key: Option<&str>, #[case] expected: &str) {
    let settings = settings([Some("g"), Some("G")], Some("m"));
    let found = key_of(&BTreeMap::new(), api_key, Some("p"), &settings);
    assert_eq!(found.as_deref(), Some(expected));
}

#[test]
fn an_empty_params_key_falls_to_the_module_key() {
    let settings = settings([Some("g"), Some("G")], Some("m"));
    let found = key_of(&BTreeMap::new(), None, Some(""), &settings);
    assert_eq!(found.as_deref(), Some("m"));
}

#[test]
fn an_empty_module_key_falls_to_google() {
    let settings = settings([Some("g"), Some("G")], Some(""));
    let found = key_of(&BTreeMap::new(), None, None, &settings);
    assert_eq!(found.as_deref(), Some("g"));
}

#[test]
fn an_empty_google_key_falls_to_gemini() {
    let settings = settings([Some(""), Some("G")], None);
    let found = key_of(&BTreeMap::new(), None, None, &settings);
    assert_eq!(found.as_deref(), Some("G"));
}

#[test]
fn gemini_is_the_last_source() {
    let settings = settings([None, Some("G")], None);
    let found = key_of(&BTreeMap::new(), None, None, &settings);
    assert_eq!(found.as_deref(), Some("G"));
}

#[rstest]
#[case::absent([None, None])]
#[case::empty([Some(""), Some("")])]
fn a_missing_api_key_names_both_variables(#[case] environment: [Option<&str>; 2]) {
    let settings = settings(environment, None);
    let error = headers_for(&BTreeMap::new(), None, None, &settings).expect_err("no key is set");
    assert_eq!(
        error.to_string(),
        "GEMINI_API_KEY or GOOGLE_API_KEY is required for Veo video generation. Set it via environment variable or pass it as api_key parameter."
    );
}

#[test]
fn the_chosen_key_and_json_type_override_the_caller_headers() {
    let settings = settings([Some("google"), None], None);
    let caller = BTreeMap::from([
        ("X-Extra".to_owned(), "1".to_owned()),
        ("Content-Type".to_owned(), "text/plain".to_owned()),
        ("content-type".to_owned(), "text/plain".to_owned()),
        ("x-goog-api-key".to_owned(), "caller".to_owned()),
    ]);
    let headers = headers_for(&caller, None, None, &settings).expect("a key is found");
    let shown = |name: &str| {
        headers
            .get(name)
            .map(|value| value.expose_secret().as_str())
    };
    assert_eq!(shown("x-goog-api-key"), Some("google"));
    assert_eq!(shown("Content-Type"), Some("application/json"));
    assert_eq!(shown("content-type"), Some("text/plain"));
    assert_eq!(shown("X-Extra"), Some("1"));
}

fn url_for(model: &str, api_base: Option<&str>, configured: &str) -> String {
    let settings = Settings {
        environment_key: None,
        generic_api_key: None,
        api_base: configured.to_owned(),
    };
    complete_url(model, api_base, &settings)
}

#[rstest]
#[case::default_base(
    "veo-3",
    None,
    DEFAULT_API_BASE,
    "https://generativelanguage.googleapis.com/v1beta/models/veo-3:predictLongRunning"
)]
#[case::configured_base(
    "veo-3",
    None,
    "https://env.example/",
    "https://env.example/v1beta/models/veo-3:predictLongRunning"
)]
#[case::explicit_base_wins(
    "veo-3",
    Some("https://mock"),
    "https://env.example",
    "https://mock/v1beta/models/veo-3:predictLongRunning"
)]
#[case::empty_explicit_base(
    "veo-3",
    Some(""),
    DEFAULT_API_BASE,
    "/v1beta/models/veo-3:predictLongRunning"
)]
#[case::empty_model("", Some(""), DEFAULT_API_BASE, "")]
#[case::empty_model_default_base("", None, "https://env.example/", "https://env.example")]
#[case::trailing_slashes(
    "veo-3",
    Some("https://mock//"),
    DEFAULT_API_BASE,
    "https://mock/v1beta/models/veo-3:predictLongRunning"
)]
#[case::every_prefix_removed(
    "gemini/gemini/veo-3",
    None,
    DEFAULT_API_BASE,
    "https://generativelanguage.googleapis.com/v1beta/models/veo-3:predictLongRunning"
)]
#[case::only_the_prefix_removed(
    "gemini/models/veo-3",
    None,
    DEFAULT_API_BASE,
    "https://generativelanguage.googleapis.com/v1beta/models/models/veo-3:predictLongRunning"
)]
fn the_complete_url_follows_litellm(
    #[case] model: &str,
    #[case] api_base: Option<&str>,
    #[case] configured: &str,
    #[case] expected: &str,
) {
    assert_eq!(url_for(model, api_base, configured), expected);
}

#[test]
fn the_default_base_builds_the_predict_long_running_url() {
    assert_eq!(
        url_for("veo-3", None, DEFAULT_API_BASE),
        "https://generativelanguage.googleapis.com/v1beta/models/veo-3:predictLongRunning"
    );
}
