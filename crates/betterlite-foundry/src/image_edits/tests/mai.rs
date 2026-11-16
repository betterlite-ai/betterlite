use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::image_edits::request::ImageEditForm;

use crate::image_edits::mai::{
    MAI_EDIT_OPENAI_PARAMS, MaiEditRequest, supported_params, transform_request,
};

use super::support::{self, JPEG, MAI, PNG};

fn file(content: &[u8]) -> Option<ImageEditFile> {
    Some(ImageEditFile {
        content: content.to_vec(),
        name: None,
    })
}

fn form(
    prompt: Option<&str>,
    image: Option<&[Option<ImageEditFile>]>,
    options: &ImageParams,
) -> ImageEditForm {
    transform_request(MaiEditRequest {
        model: MAI,
        prompt,
        image,
        options,
    })
}

pub(super) fn mai_edit_prompt_images() {
    assert_eq!(supported_params(), MAI_EDIT_OPENAI_PARAMS.as_slice());
    assert_eq!(supported_params(), &["prompt", "image", "model", "n"]);
    prompt_and_first_image();
    prompt_ordering();
    missing_values_are_not_rejected();
}

fn prompt_and_first_image() {
    let images = [None, file(PNG), file(JPEG)];
    let form = form(
        Some("paint"),
        Some(&images),
        &support::params(r#"{"image": "dropped", "mask": "dropped", "n": 2}"#),
    );
    assert_eq!(
        form.data,
        support::params(r#"{"model": "MAI-Image-2.5", "n": 2, "prompt": "paint"}"#)
    );
    assert_eq!(form.files.len(), 1);
    let (field, uploaded) = form.files.first().expect("one image file");
    assert_eq!(field, "image");
    assert_eq!(uploaded.content, PNG.to_vec());
    assert_eq!(uploaded.filename.as_deref(), Some("image.png"));
    assert_eq!(uploaded.content_type.as_deref(), Some("image/png"));

    let only_jpeg = [file(JPEG)];
    let form = form(None, Some(&only_jpeg), &ImageParams::new());
    assert_eq!(form.data, support::params(r#"{"model": "MAI-Image-2.5"}"#));
    let (_, uploaded) = form.files.first().expect("one image file");
    assert_eq!(uploaded.content_type.as_deref(), Some("image/jpeg"));
}

fn prompt_ordering() {
    let form = form(
        Some("positional"),
        None,
        &support::params(r#"{"prompt": "option", "model": "other-model"}"#),
    );
    assert_eq!(
        form.data,
        support::params(r#"{"model": "other-model", "prompt": "positional"}"#)
    );
    let form = form(None, None, &support::params(r#"{"prompt": "option"}"#));
    assert_eq!(
        form.data,
        support::params(r#"{"model": "MAI-Image-2.5", "prompt": "option"}"#)
    );
}

fn missing_values_are_not_rejected() {
    let form = form(None, None, &ImageParams::new());
    assert_eq!(form.data, support::params(r#"{"model": "MAI-Image-2.5"}"#));
    assert!(form.files.is_empty());

    let images = [None, None];
    let form = form(Some("paint"), Some(&images), &ImageParams::new());
    assert!(form.files.is_empty());

    let empty: [Option<ImageEditFile>; 0] = [];
    let form = form(Some("paint"), Some(&empty), &ImageParams::new());
    assert!(form.files.is_empty());
    assert_eq!(
        form.data,
        support::params(r#"{"model": "MAI-Image-2.5", "prompt": "paint"}"#)
    );
}
