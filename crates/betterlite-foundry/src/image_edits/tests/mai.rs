use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::image_edits::request::ImageEditForm;

use crate::image_edits::mai::{MAI_EDIT_OPENAI_PARAMS, MaiEditRequest, transform_request};

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
    transform_request(&MaiEditRequest {
        model: MAI,
        prompt,
        image,
        options,
    })
}

pub(super) fn mai_edit_prompt_images() {
    assert_eq!(&MAI_EDIT_OPENAI_PARAMS, &["prompt", "image", "model", "n"]);
    prompt_and_first_image();
    prompt_ordering();
    missing_values_are_not_rejected();
}

fn prompt_and_first_image() {
    let images = [None, file(PNG), file(JPEG)];
    let built = form(
        Some("paint"),
        Some(&images),
        &support::params(r#"{"image": "dropped", "mask": "dropped", "n": 2}"#),
    );
    assert_eq!(
        built.data,
        support::params(r#"{"model": "MAI-Image-2.5", "n": 2, "prompt": "paint"}"#)
    );
    assert_eq!(built.files.len(), 1);
    let (field, uploaded) = built.files.first().expect("one image file");
    assert_eq!(field, "image");
    assert_eq!(uploaded.content, PNG.to_vec());
    assert_eq!(uploaded.filename.as_deref(), Some("image.png"));
    assert_eq!(uploaded.content_type.as_deref(), Some("image/png"));

    let only_jpeg = [file(JPEG)];
    let jpeg_form = form(None, Some(&only_jpeg), &ImageParams::new());
    assert_eq!(
        jpeg_form.data,
        support::params(r#"{"model": "MAI-Image-2.5"}"#)
    );
    let (_, jpeg_file) = jpeg_form.files.first().expect("one image file");
    assert_eq!(jpeg_file.content_type.as_deref(), Some("image/jpeg"));
}

fn prompt_ordering() {
    let positional = form(
        Some("positional"),
        None,
        &support::params(r#"{"prompt": "option", "model": "other-model"}"#),
    );
    assert_eq!(
        positional.data,
        support::params(r#"{"model": "other-model", "prompt": "positional"}"#)
    );
    let option_form = form(None, None, &support::params(r#"{"prompt": "option"}"#));
    assert_eq!(
        option_form.data,
        support::params(r#"{"model": "MAI-Image-2.5", "prompt": "option"}"#)
    );
}

fn missing_values_are_not_rejected() {
    let unset = form(None, None, &ImageParams::new());
    assert_eq!(unset.data, support::params(r#"{"model": "MAI-Image-2.5"}"#));
    assert!(unset.files.is_empty());

    let images = [None, None];
    let nulls = form(Some("paint"), Some(&images), &ImageParams::new());
    assert!(nulls.files.is_empty());

    let empty: [Option<ImageEditFile>; 0] = [];
    let empty_list = form(Some("paint"), Some(&empty), &ImageParams::new());
    assert!(empty_list.files.is_empty());
    assert_eq!(
        empty_list.data,
        support::params(r#"{"model": "MAI-Image-2.5", "prompt": "paint"}"#)
    );
}
