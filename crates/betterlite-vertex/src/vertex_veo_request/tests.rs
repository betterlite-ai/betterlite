#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_openai::openai_video_identifiers::multipart::ReadSeek;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use crate::credential_sources::getters::VertexParams;

use super::SUPPORTED_OPENAI_PARAMS;
use super::create::{CreateForm, CreateInput, create_request};
use super::edit::{EditInput, OperationResponse, PrefetchedOperation, SourceVideo, edit_request};
use super::error::VeoRequestError;
use super::image::{B1120Mime, ImageInput, MimeReader};
use super::params::{
    MapInput, MappedParams, map_openai_params, size_aspect_ratio, size_resolution,
    supports_resolution_inference,
};
use super::url::complete_url;

mod environment;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object<const N: usize>(fields: [(&str, Json); N]) -> Object {
    fields
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

struct Catalog(BTreeMap<String, PriceRecord>);

fn catalog() -> Catalog {
    Catalog(BTreeMap::from([
        (
            "vertex_ai/veo-3.0-fast-generate-001".to_owned(),
            PriceRecord::from([(
                "output_cost_per_second_1080p".to_owned(),
                Scalar::Text("0.12".to_owned()),
            )]),
        ),
        (
            "vertex_ai/veo-2.0-generate-001".to_owned(),
            PriceRecord::from([("output_cost_per_second_1080p".to_owned(), Scalar::Null)]),
        ),
    ]))
}

impl PriceLookup for Catalog {
    fn effective_model_info(&self, model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        self.0.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.0.get(key).cloned()
    }

    fn effective_priced_model(&self, _model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        None
    }
}

#[test]
fn supported_params_match_litellm() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["model", "prompt", "input_reference", "seconds", "size"]
    );
}

#[rstest]
#[case::landscape_720("1280x720", Some("16:9"), Some("720p"))]
#[case::landscape_1080("1920x1080", Some("16:9"), Some("1080p"))]
#[case::portrait_720("720x1280", Some("9:16"), Some("720p"))]
#[case::portrait_1080("1080x1920", Some("9:16"), Some("1080p"))]
#[case::unknown_nonempty("1024x1024", Some("16:9"), None)]
#[case::empty("", None, None)]
fn size_maps_to_aspect_ratio_and_resolution(
    #[case] size: &str,
    #[case] ratio: Option<&str>,
    #[case] resolution: Option<&str>,
) {
    assert_eq!(size_aspect_ratio(size), ratio);
    assert_eq!(size_resolution(size), resolution);
}

#[rstest]
#[case::catalog_rate("veo-3.0-fast-generate-001", true)]
#[case::prefixed("vertex_ai/veo-3.0-fast-generate-001", true)]
#[case::null_rate("veo-2.0-generate-001", false)]
#[case::unknown_model("veo-9", false)]
fn resolution_inference_requires_a_1080p_catalog_rate(
    #[case] model: &str,
    #[case] supported: bool,
) {
    assert_eq!(supports_resolution_inference(model, &catalog()), supported);
}

fn map_input(model: &str) -> MapInput<'_> {
    MapInput {
        model,
        input_reference: None,
        image: None,
        parameters: None,
        resolution: None,
        seconds: None,
        size: None,
    }
}

#[test]
fn inferred_resolution_is_added_only_for_a_gated_model() {
    let mut gated = map_input("veo-3.0-fast-generate-001");
    gated.size = Some("1280x720".to_owned());
    let gated_mapped = map_openai_params(gated, &catalog());
    assert_eq!(gated_mapped.aspect_ratio.as_deref(), Some("16:9"));
    assert_eq!(gated_mapped.resolution, Some(text("720p")));

    let mut ungated = map_input("veo-2.0-generate-001");
    ungated.size = Some("1280x720".to_owned());
    let ungated_mapped = map_openai_params(ungated, &catalog());
    assert_eq!(ungated_mapped.aspect_ratio.as_deref(), Some("16:9"));
    assert_eq!(ungated_mapped.resolution, None);
}

#[test]
fn explicit_resolution_blocks_inference() {
    let mut input = map_input("veo-3.0-fast-generate-001");
    input.size = Some("1920x1080".to_owned());
    input.resolution = Some(Json::Null);
    let mapped = map_openai_params(input, &catalog());
    assert_eq!(mapped.resolution, Some(Json::Null));
}

#[test]
fn a_nested_resolution_blocks_inference_and_is_kept() {
    let mut input = map_input("veo-3.0-fast-generate-001");
    input.size = Some("1920x1080".to_owned());
    input.parameters = Some(object([("resolution", text("720p"))]));
    let mapped = map_openai_params(input, &catalog());
    assert_eq!(mapped.resolution, None);
    assert_eq!(
        mapped.parameters,
        Some(object([("resolution", text("720p"))]))
    );
    assert_eq!(mapped.aspect_ratio.as_deref(), Some("16:9"));
}

#[test]
fn a_null_nested_resolution_does_not_block_inference() {
    let mut input = map_input("veo-3.0-fast-generate-001");
    input.size = Some("1920x1080".to_owned());
    input.parameters = Some(object([("resolution", Json::Null)]));
    let mapped = map_openai_params(input, &catalog());
    assert_eq!(mapped.resolution, Some(text("1080p")));
    assert_eq!(
        mapped.parameters,
        Some(object([("resolution", Json::Null)]))
    );
}

#[test]
fn input_reference_becomes_image_before_image_does() {
    let mut input = map_input("veo-3.0-fast-generate-001");
    input.input_reference = Some(ImageInput::Json(Json::Object(object([(
        "gcsUri",
        text("gs://reference"),
    )]))));
    input.image = Some(ImageInput::Json(Json::Object(object([(
        "gcsUri",
        text("gs://image"),
    )]))));
    let mapped = map_openai_params(input, &catalog());
    let Some(ImageInput::Json(Json::Object(fields))) = mapped.image else {
        panic!("the reference should become the image");
    };
    assert_eq!(fields.get("gcsUri"), Some(&text("gs://reference")));
}

#[test]
fn an_explicit_null_input_reference_wins_as_a_null_image() {
    let mut input = map_input("veo-3.0-fast-generate-001");
    input.input_reference = Some(ImageInput::Json(Json::Null));
    input.image = Some(ImageInput::Json(text("gs://image")));
    let mapped = map_openai_params(input, &catalog());
    assert!(matches!(mapped.image, Some(ImageInput::Json(Json::Null))));
}

#[rstest]
#[case::plain("8", Some(8))]
#[case::padded(" 8 ", Some(8))]
#[case::signed("+8", Some(8))]
#[case::underscored("1_0", Some(10))]
#[case::decimal("8.0", None)]
#[case::words("eight", None)]
fn seconds_parse_as_python_integers(#[case] seconds: &str, #[case] expected: Option<i64>) {
    let mut input = map_input("veo-3.0-fast-generate-001");
    input.seconds = Some(seconds.to_owned());
    let mapped = map_openai_params(input, &catalog());
    assert_eq!(mapped.duration_seconds, expected);
}

fn mapped<'media>() -> MappedParams<'media> {
    MappedParams {
        image: None,
        parameters: None,
        resolution: None,
        aspect_ratio: None,
        duration_seconds: None,
    }
}

struct PanickingMime;

impl MimeReader for PanickingMime {
    fn mime(&self, _reader: &mut dyn ReadSeek) -> &'static str {
        panic!("a gs:// image must not be read");
    }
}

struct Probe {
    data: Vec<u8>,
    position: usize,
    seeks: usize,
}

impl Probe {
    const fn new(data: Vec<u8>, position: usize) -> Self {
        Self {
            data,
            position,
            seeks: 0,
        }
    }
}

impl Read for Probe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.data.len().saturating_sub(self.position);
        let taken = remaining.min(buffer.len());
        let source = self
            .data
            .get(self.position..self.position.saturating_add(taken))
            .unwrap_or_default();
        buffer
            .get_mut(..taken)
            .unwrap_or_default()
            .copy_from_slice(source);
        self.position = self.position.saturating_add(taken);
        Ok(taken)
    }
}

impl Seek for Probe {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks = self.seeks.saturating_add(1);
        if let SeekFrom::Start(offset) = position {
            self.position =
                usize::try_from(offset).map_err(|error| io::Error::other(error.to_string()))?;
        }
        Ok(u64::try_from(self.position).unwrap_or_default())
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(u64::try_from(self.position).unwrap_or_default())
    }
}

fn create<'input>(
    mapped: MappedParams<'input>,
    extra: &'input Object,
    mime: &dyn MimeReader,
) -> Result<CreateForm, VeoRequestError> {
    create_request(
        CreateInput {
            prompt: "a prompt",
            api_base: "https://mock/veo-3",
            mapped,
            extra,
        },
        mime,
    )
}

fn instance_image(form: &CreateForm) -> Option<&Json> {
    let Some(Json::List(instances)) = form.body.get("instances") else {
        return None;
    };
    let Some(Json::Object(first_instance)) = instances.first() else {
        return None;
    };
    first_instance.get("image")
}

fn parameters(form: &CreateForm) -> Option<&Json> {
    form.body.get("parameters")
}

#[test]
fn create_maps_a_gs_uri_without_probing_the_image() {
    let mut mapped = mapped();
    mapped.image = Some(ImageInput::Json(text("gs://bucket/a.png")));
    let extra = Object::new();
    let form = create(mapped, &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(form.url, "https://mock/veo-3:predictLongRunning");
    assert_eq!(
        instance_image(&form),
        Some(&Json::Object(object([(
            "gcsUri",
            text("gs://bucket/a.png")
        )])))
    );
    assert_eq!(parameters(&form), None);
}

#[test]
fn create_seeks_a_local_image_to_zero_and_leaves_it_consumed() {
    let data = b"\x89PNG\r\n\x1a\nlocal image".to_vec();
    let mut reader = Probe::new(data.clone(), 4);
    let mut mapped = mapped();
    mapped.image = Some(ImageInput::File(&mut reader));
    let extra = Object::new();
    let form = create(mapped, &extra, &B1120Mime).expect("the create request is built");
    assert_eq!(reader.position, data.len());
    assert_eq!(reader.seeks, 3);
    assert_eq!(
        instance_image(&form),
        Some(&Json::Object(object([
            ("bytesBase64Encoded", Json::Text(STANDARD.encode(&data))),
            ("mimeType", text("image/png")),
        ])))
    );
}

#[test]
fn create_passes_a_vertex_image_dict_through() {
    let mut mapped = mapped();
    let image = object([
        ("bytesBase64Encoded", text("abc")),
        ("mimeType", text("image/jpeg")),
    ]);
    mapped.image = Some(ImageInput::Json(Json::Object(image.clone())));
    let extra = Object::new();
    let form = create(mapped, &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(instance_image(&form), Some(&Json::Object(image)));
}

#[test]
fn create_rejects_a_plain_string_image() {
    let mut mapped = mapped();
    mapped.image = Some(ImageInput::Json(text("https://example.invalid/a.png")));
    let extra = Object::new();
    let error =
        create(mapped, &extra, &PanickingMime).expect_err("a plain URL is not a Vertex image");
    assert!(matches!(error, VeoRequestError::UnsupportedImage { .. }));
}

#[test]
fn create_rejects_an_image_of_another_shape() {
    let mut mapped = mapped();
    mapped.image = Some(ImageInput::Json(number(3)));
    let extra = Object::new();
    let error = create(mapped, &extra, &PanickingMime).expect_err("a number is not a Vertex image");
    assert!(matches!(error, VeoRequestError::ImageShape));
}

#[test]
fn create_keeps_an_explicit_null_image_among_the_parameters() {
    let mut mapped = mapped();
    mapped.image = Some(ImageInput::Json(Json::Null));
    let extra = Object::new();
    let form = create(mapped, &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(instance_image(&form), None);
    assert_eq!(
        parameters(&form),
        Some(&Json::Object(object([("image", Json::Null)])))
    );
}

#[test]
fn create_lets_an_instances_dict_suppress_image_conversion() {
    let mut mapped = mapped();
    mapped.image = Some(ImageInput::Json(Json::Object(object([(
        "gcsUri",
        text("gs://image"),
    )]))));
    mapped.aspect_ratio = Some("9:16".to_owned());
    let extra = object([
        (
            "instances",
            Json::Object(object([("prompt", text("the instance prompt"))])),
        ),
        ("aspectRatio", text("16:9")),
    ]);
    let form = create(mapped, &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(instance_image(&form), None);
    assert_eq!(
        parameters(&form),
        Some(&Json::Object(object([
            (
                "image",
                Json::Object(object([("gcsUri", text("gs://image"))]))
            ),
            ("aspectRatio", text("16:9")),
        ])))
    );
}

#[test]
fn create_refuses_a_file_image_it_cannot_flatten() {
    let mut reader = Probe::new(b"\x89PNG\r\n\x1a\n".to_vec(), 0);
    let mut mapped = mapped();
    mapped.image = Some(ImageInput::File(&mut reader));
    let extra = object([(
        "instances",
        Json::Object(object([("prompt", text("the instance prompt"))])),
    )]);
    let error = create(mapped, &extra, &B1120Mime).expect_err("a stream cannot be an option");
    assert!(matches!(error, VeoRequestError::ImageInParameters));
    assert_eq!(reader.seeks, 0);
}

#[test]
fn create_flattens_a_non_object_instances_option() {
    let mut mapped = mapped();
    mapped.image = Some(ImageInput::Json(text("gs://image")));
    let extra = object([("instances", text("not a dict"))]);
    let form = create(mapped, &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(
        instance_image(&form),
        Some(&Json::Object(object([("gcsUri", text("gs://image"))])))
    );
    assert_eq!(
        parameters(&form),
        Some(&Json::Object(object([("instances", text("not a dict"))])))
    );
}

#[test]
fn create_flattens_nested_parameters_then_flat_options() {
    let mut mapped = mapped();
    mapped.parameters = Some(object([
        ("personGeneration", text("allow")),
        ("aspectRatio", text("9:16")),
    ]));
    mapped.aspect_ratio = Some("1:1".to_owned());
    let extra = object([("durationSeconds", number(8))]);
    let form = create(mapped, &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(
        parameters(&form),
        Some(&Json::Object(object([
            ("personGeneration", text("allow")),
            ("aspectRatio", text("1:1")),
            ("durationSeconds", number(8)),
        ])))
    );
}

#[test]
fn extra_body_options_override_the_mapped_ones() {
    let mut mapped = mapped();
    mapped.parameters = Some(object([
        ("personGeneration", text("deny")),
        ("aspectRatio", text("9:16")),
    ]));
    mapped.aspect_ratio = Some("9:16".to_owned());
    let extra = object([
        (
            "parameters",
            Json::Object(object([("personGeneration", text("allow"))])),
        ),
        ("aspectRatio", text("1:1")),
    ]);
    let form = create(mapped, &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(
        parameters(&form),
        Some(&Json::Object(object([
            ("personGeneration", text("allow")),
            ("aspectRatio", text("1:1")),
        ])))
    );
}

#[test]
fn an_extra_body_image_overrides_the_mapped_image() {
    let mut mapped = mapped();
    mapped.image = Some(ImageInput::Json(Json::Object(object([(
        "gcsUri",
        text("gs://mapped"),
    )]))));
    let extra = object([("image", text("gs://extra"))]);
    let form = create(mapped, &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(
        instance_image(&form),
        Some(&Json::Object(object([("gcsUri", text("gs://extra"))])))
    );
    assert_eq!(parameters(&form), None);
}

#[test]
fn an_extra_body_key_is_never_sent_as_a_parameter() {
    let extra = object([(
        "extra_body",
        Json::Object(object([("personGeneration", text("allow"))])),
    )]);
    let form = create(mapped(), &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(parameters(&form), None);
}

#[test]
fn create_omits_an_empty_parameters_block() {
    let extra = Object::new();
    let form = create(mapped(), &extra, &PanickingMime).expect("the create request is built");
    assert_eq!(parameters(&form), None);
    assert_eq!(
        form.body.get("instances"),
        Some(&Json::List(vec![Json::Object(object([(
            "prompt",
            text("a prompt")
        )]))]))
    );
}

const OPERATION: &str =
    "projects/p/locations/l/publishers/google/models/veo-3.0-generate-001/operations/op-1";

fn source_video(
    gcs_uri: Option<&str>,
    encoded: Option<&str>,
    mime_type: Option<&str>,
) -> SourceVideo {
    SourceVideo {
        gcs_uri: gcs_uri.map(str::to_owned),
        bytes_base64_encoded: encoded.map(str::to_owned),
        mime_type: mime_type.map(str::to_owned),
    }
}

fn prefetched(videos: Vec<SourceVideo>, done: bool) -> PrefetchedOperation {
    PrefetchedOperation {
        done,
        response: OperationResponse { videos },
    }
}

fn edit(
    prefetched: Option<&PrefetchedOperation>,
    extra_body: Option<&Object>,
) -> Result<super::edit::EditForm, VeoRequestError> {
    let video_id =
        encode_video_id_with_provider(OPERATION, "vertex_ai", Some("veo-3.0-generate-001"));
    edit_request(&EditInput {
        prompt: "an edit",
        video_id: &video_id,
        api_base: "https://mock/",
        extra_body,
        prefetched,
    })
}

#[test]
fn edit_requires_prefetched_source_data() {
    let error = edit(None, None).expect_err("an edit needs the prefetched operation");
    assert!(matches!(error, VeoRequestError::PrefetchRequired));
}

#[rstest]
#[case::absent_done("{}")]
#[case::false_flag(r#"{"done": false}"#)]
#[case::null_done(r#"{"done": null}"#)]
fn edit_requires_a_completed_source(#[case] source: &str) {
    let operation: PrefetchedOperation = serde_json::from_str(source).expect("it decodes");
    let error = edit(Some(&operation), None).expect_err("the source is unfinished");
    assert!(matches!(error, VeoRequestError::SourceNotDone));
}

#[rstest]
#[case::empty_videos(r#"{"done": true, "response": {"videos": []}}"#)]
#[case::absent_videos(r#"{"done": true, "response": {}}"#)]
#[case::null_videos(r#"{"done": true, "response": {"videos": null}}"#)]
#[case::absent_response(r#"{"done": true}"#)]
#[case::null_response(r#"{"done": true, "response": null}"#)]
fn edit_requires_at_least_one_source_video(#[case] source: &str) {
    let operation: PrefetchedOperation = serde_json::from_str(source).expect("it decodes");
    let error = edit(Some(&operation), None).expect_err("there is no video");
    assert!(matches!(error, VeoRequestError::NoVideos));
}

#[test]
fn edit_prefers_gcs_uri_by_key_presence_even_when_empty() {
    let operation = prefetched(
        vec![source_video(Some(""), Some("abc"), Some("video/webm"))],
        true,
    );
    let form = edit(Some(&operation), None).expect("the edit request is built");
    assert_eq!(
        form.body.get("instances"),
        Some(&Json::List(vec![Json::Object(object([
            ("prompt", text("an edit")),
            ("video", Json::Object(object([("gcsUri", text(""))]))),
        ]))]))
    );
}

#[rstest]
#[case::default_mime(None, "video/mp4")]
#[case::given_mime(Some("video/webm"), "video/webm")]
#[case::empty_mime(Some(""), "")]
fn edit_base64_sources_take_the_declared_mime(
    #[case] mime_type: Option<&str>,
    #[case] expected: &str,
) {
    let operation = prefetched(vec![source_video(None, Some("abc"), mime_type)], true);
    let form = edit(Some(&operation), None).expect("the edit request is built");
    let Some(Json::List(instances)) = form.body.get("instances") else {
        panic!("the edit request has instances");
    };
    let Some(Json::Object(first_instance)) = instances.first() else {
        panic!("the edit request has one instance");
    };
    assert_eq!(
        first_instance.get("video"),
        Some(&Json::Object(object([
            ("bytesBase64Encoded", text("abc")),
            ("mimeType", text(expected)),
        ])))
    );
}

#[test]
fn edit_rejects_a_source_without_media() {
    let operation = prefetched(vec![source_video(None, None, None)], true);
    let error = edit(Some(&operation), None).expect_err("the source has no media");
    assert!(matches!(error, VeoRequestError::MissingSourceVideo));
}

#[test]
fn edit_builds_the_model_url_from_the_encoded_operation_name() {
    let operation = prefetched(vec![source_video(Some("gs://v"), None, None)], true);
    let form = edit(Some(&operation), None).expect("the edit request is built");
    assert_eq!(
        form.url,
        "https://mock/veo-3.0-generate-001:predictLongRunning"
    );
}

#[test]
fn edit_keeps_the_url_shape_for_a_plain_video_id() {
    let operation = prefetched(vec![source_video(Some("gs://v"), None, None)], true);
    let form = edit_request(&EditInput {
        prompt: "an edit",
        video_id: "plain-id",
        api_base: "https://mock/",
        extra_body: None,
        prefetched: Some(&operation),
    })
    .expect("the edit request is built");
    assert_eq!(form.url, "https://mock/:predictLongRunning");
}

#[rstest]
#[case::full_name(
    "projects/p/locations/l/publishers/google/models/veo-3/operations/op",
    "veo-3"
)]
#[case::extra_components(
    "projects/p/locations/l/publishers/google/models/veo-3/operations/op/extra",
    "veo-3"
)]
#[case::seven_components("projects/p/locations/l/publishers/google/models", "")]
#[case::empty("", "")]
fn edit_takes_the_model_from_element_seven_of_the_operation_name(
    #[case] video_id: &str,
    #[case] model: &str,
) {
    let operation = prefetched(vec![source_video(Some("gs://v"), None, None)], true);
    let form = edit_request(&EditInput {
        prompt: "an edit",
        video_id,
        api_base: "https://mock/",
        extra_body: None,
        prefetched: Some(&operation),
    })
    .expect("the edit request is built");
    assert_eq!(form.url, format!("https://mock/{model}:predictLongRunning"));
}

#[test]
fn edit_flattens_extra_body_parameters_then_flat_options() {
    let operation = prefetched(vec![source_video(Some("gs://v"), None, None)], true);
    let extra = object([
        (
            "parameters",
            Json::Object(object([
                ("personGeneration", text("allow")),
                ("aspectRatio", text("9:16")),
            ])),
        ),
        ("aspectRatio", text("16:9")),
    ]);
    let form = edit(Some(&operation), Some(&extra)).expect("the edit request is built");
    assert_eq!(
        parameters_of_edit(&form),
        Some(&Json::Object(object([
            ("personGeneration", text("allow")),
            ("aspectRatio", text("16:9")),
        ])))
    );
}

#[test]
fn edit_drops_a_non_object_parameters_block() {
    let operation = prefetched(vec![source_video(Some("gs://v"), None, None)], true);
    let extra = object([
        ("parameters", text("not a dict")),
        ("aspectRatio", text("16:9")),
    ]);
    let form = edit(Some(&operation), Some(&extra)).expect("the edit request is built");
    assert_eq!(
        parameters_of_edit(&form),
        Some(&Json::Object(object([("aspectRatio", text("16:9"))])))
    );
}

#[test]
fn edit_omits_an_empty_extra_body() {
    let operation = prefetched(vec![source_video(Some("gs://v"), None, None)], true);
    let empty = Object::new();
    let form = edit(Some(&operation), Some(&empty)).expect("the edit request is built");
    assert_eq!(parameters_of_edit(&form), None);
}

fn parameters_of_edit(form: &super::edit::EditForm) -> Option<&Json> {
    form.body.get("parameters")
}

fn vertex_params<'settings>(
    project: Option<&'settings str>,
    location: Option<&'settings str>,
) -> VertexParams<'settings> {
    VertexParams {
        project,
        ai_project: None,
        location,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    }
}

#[rstest]
#[case::absent_location(None)]
#[case::empty_location(Some(""))]
fn complete_url_defaults_the_location_and_removes_the_provider_prefix(
    #[case] location: Option<&str>,
) {
    let settings = VertexSettings::default();
    let url = complete_url(
        "vertex_ai/veo-3.0-generate-001",
        None,
        vertex_params(Some("project-1"), location),
        &settings,
    )
    .expect("the URL is built");
    assert_eq!(
        url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/project-1/locations/us-central1/publishers/google/models/veo-3.0-generate-001"
    );
}

#[test]
fn complete_url_removes_every_provider_prefix() {
    let settings = VertexSettings::default();
    let url = complete_url(
        "vertex_ai/vertex_ai/veo-3.0-generate-001",
        None,
        vertex_params(Some("project-1"), None),
        &settings,
    )
    .expect("the URL is built");
    assert!(url.ends_with("/models/veo-3.0-generate-001"));
}

#[test]
fn complete_url_uses_a_custom_base_and_the_configured_location() {
    let settings = VertexSettings::default();
    let url = complete_url(
        "veo-3.0-generate-001",
        Some("https://mock//"),
        vertex_params(Some("project-1"), Some("europe-west4")),
        &settings,
    )
    .expect("the URL is built");
    assert_eq!(
        url,
        "https://mock/v1/projects/project-1/locations/europe-west4/publishers/google/models/veo-3.0-generate-001"
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn complete_url_requires_a_configured_project(#[case] project: Option<&str>) {
    let settings = VertexSettings::default();
    let error = complete_url(
        "veo-3.0-generate-001",
        None,
        vertex_params(project, None),
        &settings,
    )
    .expect_err("the project is required");
    assert!(matches!(error, VeoRequestError::MissingProject));
}

#[test]
fn complete_url_prefers_the_params_alias_over_the_settings() {
    let settings = VertexSettings {
        project: Some("settings-project".to_owned()),
        location: None,
        credentials: None,
        adc: AdcEnvironment::default(),
    };
    let aliased = VertexParams {
        project: None,
        ai_project: Some("alias-project"),
        location: None,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    };
    let url =
        complete_url("veo-3.0-generate-001", None, aliased, &settings).expect("the URL is built");
    assert!(url.contains("/projects/alias-project/"));
    let from_settings = complete_url(
        "veo-3.0-generate-001",
        None,
        vertex_params(None, None),
        &settings,
    )
    .expect("the URL is built");
    assert!(from_settings.contains("/projects/settings-project/"));
}

#[test]
fn complete_url_rejects_a_project_that_escapes_the_path() {
    let settings = VertexSettings::default();
    let error = complete_url(
        "veo-3.0-generate-001",
        None,
        vertex_params(Some("../other"), None),
        &settings,
    )
    .expect_err("the project cannot be a path");
    assert!(matches!(error, VeoRequestError::Project(_)));
}

#[test]
fn complete_url_rejects_an_invalid_location_even_with_a_custom_base() {
    let settings = VertexSettings::default();
    let error = complete_url(
        "veo-3.0-generate-001",
        Some("https://mock/"),
        vertex_params(Some("project-1"), Some("us central")),
        &settings,
    )
    .expect_err("the location cannot be a path");
    assert!(matches!(error, VeoRequestError::Location(_)));
}
