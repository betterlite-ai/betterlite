use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::image_edits::request::ImageEditForm;

use crate::image_edits::error::ImageEditError;
use crate::image_edits::flux2::{
    DEFAULT_REFERENCE_LIMIT, FLEX_REFERENCE_LIMIT, Flux2EditRequest, map_edit_params,
    transform_request,
};
use crate::image_edits::image_source::ImageSource;
use crate::image_generation::error::FoundryImageError;
use crate::image_generation::flux::{FLUX2_OPENAI_PARAMS, supported_params};

use super::support::{self, FLEX, FLUX2, JPEG, PNG};

fn form(
    model: &str,
    prompt: Option<&str>,
    sources: Option<&mut Vec<ImageSource>>,
    options: &ImageParams,
) -> Result<ImageEditForm, ImageEditError> {
    transform_request(Flux2EditRequest {
        model,
        prompt,
        images: sources.map(Vec::as_mut_slice),
        options,
    })
}

fn bytes_sources(count: usize) -> Vec<ImageSource> {
    (0..count)
        .map(|index| {
            let mut content = PNG.to_vec();
            if index > 0 {
                content.push(u8::try_from(index).expect("a small index"));
            }
            ImageSource::Bytes(content)
        })
        .collect()
}

pub(super) fn flux_json_images() {
    assert_eq!(supported_params(FLUX2), FLUX2_OPENAI_PARAMS.as_slice());
    mapped_options();
    required_fields();
    reference_limits();
    field_numbering_and_overlay();
    bytes_are_inline();
}

fn mapped_options() {
    let filtered = map_edit_params(
        &support::params(r#"{"input_image": null, "n": 2, "size": "1024x1024"}"#),
        FLUX2,
        false,
    )
    .unwrap();
    assert_eq!(
        filtered,
        support::params(r#"{"num_images": 2, "width": 1024, "height": 1024}"#)
    );

    let unsupported = map_edit_params(&support::params(r#"{"style": "vivid"}"#), FLUX2, false)
        .expect_err("an unsupported parameter");
    assert!(matches!(
        unsupported,
        ImageEditError::Params(FoundryImageError::UnsupportedParams { .. })
    ));
    assert_eq!(unsupported.status_code(), Some(400));
    assert_eq!(unsupported.llm_provider(), Some("azure_ai"));

    let dropped = map_edit_params(&support::params(r#"{"style": "vivid"}"#), FLUX2, true).unwrap();
    assert!(dropped.is_empty());

    let bad_size = map_edit_params(&support::params(r#"{"size": "bad"}"#), FLUX2, false)
        .expect_err("an invalid size");
    assert!(matches!(
        bad_size,
        ImageEditError::Params(FoundryImageError::InvalidSize { .. })
    ));
    assert_eq!(bad_size.status_code(), Some(400));

    let bad_count =
        map_edit_params(&support::params(r#"{"n": "x"}"#), FLUX2, false).expect_err("a bad count");
    assert!(matches!(
        bad_count,
        ImageEditError::Params(FoundryImageError::WholeNumber { .. })
    ));
    assert_eq!(bad_count.status_code(), None);
}

fn required_fields() {
    let mut one = bytes_sources(1);
    let no_prompt = form(FLUX2, None, Some(&mut one), &ImageParams::new()).unwrap_err();
    assert!(matches!(no_prompt, ImageEditError::Flux2PromptRequired));

    let no_image = form(FLUX2, Some("draw"), None, &ImageParams::new()).unwrap_err();
    assert!(matches!(no_image, ImageEditError::Flux2ImageRequired));

    let mut empty: Vec<ImageSource> = Vec::new();
    let no_refs = form(FLUX2, Some("draw"), Some(&mut empty), &ImageParams::new()).unwrap_err();
    assert!(matches!(no_refs, ImageEditError::Flux2NoReferenceImages));
}

fn reference_limits() {
    for (model, limit) in [
        (FLUX2, DEFAULT_REFERENCE_LIMIT),
        (FLEX, FLEX_REFERENCE_LIMIT),
    ] {
        let mut at_limit = bytes_sources(limit);
        let built = form(
            model,
            Some("draw"),
            Some(&mut at_limit),
            &ImageParams::new(),
        )
        .expect("the reference limit");
        assert_eq!(built.data.len(), limit + 2, "{model}");

        let mut over_limit = bytes_sources(limit + 1);
        let error = form(
            model,
            Some("draw"),
            Some(&mut over_limit),
            &ImageParams::new(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ImageEditError::Flux2TooManyReferenceImages { limit: matched, .. } if matched == limit
        ));
        assert!(error.to_string().contains(&format!("at most {limit}")));
    }
}

fn field_numbering_and_overlay() {
    let mut three = bytes_sources(3);
    let built = form(
        FLUX2,
        Some("draw"),
        Some(&mut three),
        &support::params(r#"{"num_images": 2}"#),
    )
    .unwrap();
    let names: Vec<&str> = built.data.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "prompt",
            "model",
            "input_image",
            "input_image_2",
            "input_image_3",
            "num_images"
        ]
    );
    assert_eq!(
        built.data.get("input_image"),
        Some(&Json::Text(support::encoded(PNG)))
    );
    let mut second = PNG.to_vec();
    second.push(1);
    assert_eq!(
        built.data.get("input_image_2"),
        Some(&Json::Text(support::encoded(&second)))
    );
    assert!(built.files.is_empty());

    let overlay = support::params(r#"{"prompt": "over", "model": "over", "input_image": "over"}"#);
    let mut one = bytes_sources(1);
    let overlaid = form(FLUX2, Some("draw"), Some(&mut one), &overlay).unwrap();
    assert_eq!(overlaid.data, overlay);
}

fn bytes_are_inline() {
    let mut sources = vec![ImageSource::Bytes(JPEG.to_vec())];
    let form = form(FLUX2, Some("draw"), Some(&mut sources), &ImageParams::new()).unwrap();
    assert_eq!(
        form.data.get("input_image"),
        Some(&Json::Text(support::encoded(JPEG)))
    );
    assert!(form.files.is_empty());
}
