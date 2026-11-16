use betterlite_openai::image_generation::family::GPT_IMAGE_PARAMS;

use super::support::{mapped, object};
use crate::image_generation::error::FoundryImageError;
use crate::image_generation::flux::{
    FLUX2_DROPPED_OPENAI_PARAMS, FLUX2_OPENAI_PARAMS, supported_params,
};

const FLUX2: &str = "azure_ai/FLUX.2-pro";
const DROPPED: &str = r#"{"background":"opaque","moderation":"low","output_compression":80,"quality":"high","user":"u-1"}"#;
const NUM_TWO: &str = r#"{"num_images":2}"#;
const NUM_THREE: &str = r#"{"num_images":3}"#;
const COUNT_AND_SEED: &str = r#"{"num_images":2,"seed":7}"#;
const INTEGER_TEXTS: &str =
    r#"{"width":"1024","height":"0_768","steps":" 8 ","safety_tolerance":"+4"}"#;
const INTEGER_VALUES: &str = r#"{"width":1024,"height":768,"steps":8,"safety_tolerance":4}"#;
const HUGE_SEED_TEXT: &str = r#"{"seed":"123456789012345678901234567890"}"#;
const HUGE_SEED: &str = r#"{"seed":123456789012345678901234567890}"#;
const GUIDANCE_TWO: &str = r#"{"guidance":2.0}"#;
const GUIDANCE_TEN_AND_HALF: &str = r#"{"guidance":10.5}"#;
const PASSTHROUGH: &str = r#"{"output_format":"PNG","aspect_ratio":"16:9"}"#;

pub(super) fn flux_size_parameter_map() {
    advertised_params();
    dropped_fields();
    count_fields();
    numeric_fields();
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
    assert_eq!(
        mapped(r#"{"style":"vivid","n":"2"}"#, "{}", FLUX2, true).expect("dropped"),
        object(NUM_TWO)
    );
}

fn expect_mapped(non_default: &str, optional: &str, expected: &str) {
    assert_eq!(
        mapped(non_default, optional, FLUX2, false).expect("mapped"),
        object(expected)
    );
}

fn expect_whole_number(value: &str) {
    let text = format!(r#"{{"num_images":{value:?}}}"#);
    assert_eq!(
        mapped(&text, "{}", FLUX2, false).expect_err("a whole number"),
        FoundryImageError::WholeNumber {
            field: String::from("num_images"),
            value: String::from(value),
        }
    );
}

fn malformed_numbers() {
    for value in ["", "  ", "+", "-", "_1", "1_", "1__2", "1e3"] {
        expect_whole_number(value);
    }
}

fn count_fields() {
    expect_mapped(r#"{"n":"2"}"#, "{}", NUM_TWO);
    expect_mapped(r#"{"n":2}"#, "{}", NUM_TWO);
    expect_mapped(r#"{"num_images":"3"}"#, "{}", NUM_THREE);
    expect_mapped(
        r#"{"n":"2"}"#,
        r#"{"num_images":1,"seed":7}"#,
        COUNT_AND_SEED,
    );
}

fn numeric_fields() {
    expect_mapped(INTEGER_TEXTS, "{}", INTEGER_VALUES);
    expect_mapped(r#"{"seed":"-42"}"#, "{}", r#"{"seed":-42}"#);
    expect_mapped(r#"{"seed":"-0"}"#, "{}", r#"{"seed":0}"#);
    expect_mapped(HUGE_SEED_TEXT, "{}", HUGE_SEED);
    expect_mapped(PASSTHROUGH, "{}", PASSTHROUGH);
}

fn guidance_field() {
    expect_mapped(r#"{"guidance":"3.5"}"#, "{}", r#"{"guidance":3.5}"#);
    expect_mapped(r#"{"guidance":"2"}"#, "{}", GUIDANCE_TWO);
    expect_mapped(r#"{"guidance":" 1_0.5 "}"#, "{}", GUIDANCE_TEN_AND_HALF);
}

fn conversion_errors() {
    let count = mapped(r#"{"n":"two"}"#, "{}", FLUX2, false).expect_err("a whole number");
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
    let tolerance =
        mapped(r#"{"safety_tolerance":"6.5"}"#, "{}", FLUX2, false).expect_err("a whole number");
    assert_eq!(
        tolerance,
        FoundryImageError::WholeNumber {
            field: String::from("safety_tolerance"),
            value: String::from("6.5"),
        }
    );
    let guidance = mapped(r#"{"guidance":"fast"}"#, "{}", FLUX2, false).expect_err("a number");
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
    malformed_numbers();
    assert_eq!(
        mapped(r#"{"guidance":""}"#, "{}", FLUX2, false).expect_err("a number"),
        FoundryImageError::Decimal {
            field: String::from("guidance"),
            value: String::new(),
        }
    );
}
