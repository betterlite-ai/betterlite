use betterlite_openai::image_generation::family::GPT_IMAGE_PARAMS;

use super::support::{failure, mapped, object};
use crate::image_generation::error::FoundryImageError;
use crate::image_generation::flux::{
    FLUX2_DROPPED_OPENAI_PARAMS, FLUX2_OPENAI_PARAMS, supported_params,
};

const FLUX2: &str = "azure_ai/FLUX.2-pro";
const DROPPED: &str = r#"{"background":"opaque","moderation":"low","output_compression":80,"quality":"high","user":"u-1"}"#;

pub(super) fn flux_size_parameter_map() {
    advertised_params();
    dropped_fields();
    count_fields();
    numeric_fields();
    passthrough_fields();
    guidance_field();
    conversion_errors();
}

fn advertised_params() {
    assert_eq!(supported_params(FLUX2), &FLUX2_OPENAI_PARAMS);
    assert_eq!(supported_params(FLUX2).len(), 16);
    assert!(FLUX2_OPENAI_PARAMS.starts_with(&[
        "n",
        "size",
        "output_format",
        "seed",
        "safety_tolerance",
        "aspect_ratio",
        "width",
        "height",
        "num_images",
        "guidance",
        "steps",
    ]));
    assert!(FLUX2_OPENAI_PARAMS.ends_with(&FLUX2_DROPPED_OPENAI_PARAMS));
    assert_eq!(FLUX2_DROPPED_OPENAI_PARAMS.len(), 5);
    assert_eq!(supported_params("gpt-image-1"), &GPT_IMAGE_PARAMS);
}

fn dropped_fields() {
    assert_eq!(
        mapped(DROPPED, "{}", FLUX2, false).expect("dropped silently"),
        object("{}")
    );
    assert_eq!(
        mapped(DROPPED, "{}", FLUX2, true).expect("dropped silently"),
        object("{}")
    );
}

fn count_fields() {
    assert_eq!(
        mapped(r#"{"n":"2"}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"num_images":2}"#)
    );
    assert_eq!(
        mapped(r#"{"n":2}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"num_images":2}"#)
    );
    assert_eq!(
        mapped(r#"{"num_images":"3"}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"num_images":3}"#)
    );
    assert_eq!(
        mapped(r#"{"n":"2"}"#, r#"{"num_images":1,"seed":7}"#, FLUX2, false,).expect("mapped"),
        object(r#"{"num_images":2,"seed":7}"#)
    );
    assert_eq!(
        mapped(r#"{"style":"vivid","n":"2"}"#, "{}", FLUX2, true).expect("dropped"),
        object(r#"{"num_images":2}"#)
    );
}

fn numeric_fields() {
    assert_eq!(
        mapped(
            r#"{"width":"1024","height":"0_768","steps":" 8 ","safety_tolerance":"+4"}"#,
            "{}",
            FLUX2,
            false,
        )
        .expect("mapped"),
        object(r#"{"width":1024,"height":768,"steps":8,"safety_tolerance":4}"#)
    );
    assert_eq!(
        mapped(r#"{"seed":"-42"}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"seed":-42}"#)
    );
    assert_eq!(
        mapped(r#"{"seed":"-0"}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"seed":0}"#)
    );
    assert_eq!(
        mapped(
            r#"{"seed":"123456789012345678901234567890"}"#,
            "{}",
            FLUX2,
            false,
        )
        .expect("mapped"),
        object(r#"{"seed":123456789012345678901234567890}"#)
    );
}

fn passthrough_fields() {
    assert_eq!(
        mapped(
            r#"{"output_format":"PNG","aspect_ratio":"16:9"}"#,
            "{}",
            FLUX2,
            false,
        )
        .expect("mapped"),
        object(r#"{"output_format":"PNG","aspect_ratio":"16:9"}"#)
    );
}

fn guidance_field() {
    assert_eq!(
        mapped(r#"{"guidance":"3.5"}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"guidance":3.5}"#)
    );
    assert_eq!(
        mapped(r#"{"guidance":"2"}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"guidance":2.0}"#)
    );
    assert_eq!(
        mapped(r#"{"guidance":" 1_0.5 "}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"guidance":10.5}"#)
    );
}

fn conversion_errors() {
    let count = failure(r#"{"n":"two"}"#, "{}", FLUX2, false);
    assert_eq!(
        count,
        FoundryImageError::WholeNumber {
            field: String::from("num_images"),
            value: String::from("two"),
        }
    );
    assert_eq!(count.status_code(), None);
    assert_eq!(count.llm_provider(), None);
    assert_eq!(
        count.to_string(),
        "parameter num_images must be a whole number, got two"
    );
    let tolerance = failure(r#"{"safety_tolerance":"6.5"}"#, "{}", FLUX2, false);
    assert_eq!(
        tolerance,
        FoundryImageError::WholeNumber {
            field: String::from("safety_tolerance"),
            value: String::from("6.5"),
        }
    );
    let guidance = failure(r#"{"guidance":"fast"}"#, "{}", FLUX2, false);
    assert_eq!(
        guidance,
        FoundryImageError::Decimal {
            field: String::from("guidance"),
            value: String::from("fast"),
        }
    );
    assert_eq!(guidance.status_code(), None);
    assert_eq!(guidance.llm_provider(), None);
    assert_eq!(
        guidance.to_string(),
        "parameter guidance must be a number, got fast"
    );
}
