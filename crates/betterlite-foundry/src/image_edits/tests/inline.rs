use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use crate::image_edits::error::ImageEditError;
use crate::image_edits::flux2::{Flux2EditRequest, transform_request};
use crate::image_edits::image_source::{ImageSource, convert_image_to_base64};

use super::support::{self, FLUX2, PNG};

fn reference_field(sources: &mut [ImageSource]) -> String {
    let form = transform_request(Flux2EditRequest {
        model: FLUX2,
        prompt: Some("draw"),
        images: Some(sources),
        options: &ImageParams::new(),
    })
    .expect("a built edit form");
    let Some(Json::Text(encoded)) = form.data.get("input_image") else {
        panic!("the first reference is missing or not text");
    };
    encoded.clone()
}

pub(super) fn flux_inline_base64() {
    let mut bytes = ImageSource::Bytes(PNG.to_vec());
    assert_eq!(
        convert_image_to_base64(&mut bytes).unwrap(),
        support::encoded(PNG)
    );
    assert_eq!(bytes, ImageSource::Bytes(PNG.to_vec()));

    let mut buffered = ImageSource::Buffered {
        content: PNG.to_vec(),
        position: 2,
    };
    assert_eq!(
        convert_image_to_base64(&mut buffered).unwrap(),
        support::encoded(&PNG[2..])
    );
    assert_eq!(
        buffered,
        ImageSource::Buffered {
            content: PNG.to_vec(),
            position: 0
        }
    );

    let mut empty = ImageSource::Bytes(Vec::new());
    assert_eq!(convert_image_to_base64(&mut empty).unwrap(), "");

    let mut beyond = ImageSource::Reader {
        content: PNG.to_vec(),
        position: 99,
    };
    assert_eq!(convert_image_to_base64(&mut beyond).unwrap(), "");
    buffered_rewinds_between_requests();
    reader_keeps_its_position();
    unsupported_sources_are_refused();
}

pub(super) fn buffered_rewinds_between_requests() {
    let mut sources = vec![ImageSource::Buffered {
        content: PNG.to_vec(),
        position: 2,
    }];
    assert_eq!(reference_field(&mut sources), support::encoded(&PNG[2..]));
    assert_eq!(reference_field(&mut sources), support::encoded(PNG));
}

pub(super) fn reader_keeps_its_position() {
    let mut sources = vec![ImageSource::Reader {
        content: PNG.to_vec(),
        position: 2,
    }];
    assert_eq!(reference_field(&mut sources), support::encoded(&PNG[2..]));
    assert_eq!(reference_field(&mut sources), "");
}

pub(super) fn unsupported_sources_are_refused() {
    let mut text = ImageSource::Unsupported {
        kind: String::from("str"),
    };
    let error = convert_image_to_base64(&mut text).unwrap_err();
    assert!(matches!(error, ImageEditError::UnsupportedImageType { .. }));
    assert!(error.to_string().contains("str"));

    // a tuple is one file-like input, not a list of references
    let mut tuple = vec![ImageSource::Unsupported {
        kind: String::from("tuple"),
    }];
    let tuple_error = transform_request(Flux2EditRequest {
        model: FLUX2,
        prompt: Some("draw"),
        images: Some(tuple.as_mut_slice()),
        options: &ImageParams::new(),
    })
    .unwrap_err();
    assert!(tuple_error.to_string().contains("tuple"));
}
