use betterlite_openai::image_generation::family::{GPT_IMAGE_PARAMS, ImageParamError};

use super::support::{mapped, object};
use crate::image_generation::error::FoundryImageError;
use crate::image_generation::flux::supported_params;

const FLUX1: &str = "azure_ai/FLUX-1-schnell";

pub(super) fn flux_non_v2_policy() {
    gpt_allowlist();
    gpt_mapping();
    gpt_optional_fields();
    gpt_unsupported();
}

fn gpt_allowlist() {
    assert_eq!(supported_params(FLUX1), &GPT_IMAGE_PARAMS);
    assert_eq!(supported_params("flux-1-schnell"), &GPT_IMAGE_PARAMS);
    assert_eq!(supported_params("gpt-image-1"), &GPT_IMAGE_PARAMS);
}

fn gpt_mapping() {
    assert_eq!(
        mapped(
            r#"{"size":"1024x1024","quality":"high","n":2}"#,
            "{}",
            FLUX1,
            false,
        )
        .expect("delegated"),
        object(r#"{"size":"1024x1024","quality":"high","n":2}"#)
    );
    assert_eq!(
        mapped(r#"{"size":"auto"}"#, "{}", FLUX1, false).expect("delegated"),
        object(r#"{"size":"auto"}"#)
    );
    assert_eq!(
        mapped(r#"{"size":"bad"}"#, "{}", FLUX1, false).expect("delegated"),
        object(r#"{"size":"bad"}"#)
    );
}

fn gpt_optional_fields() {
    assert_eq!(
        mapped(
            r#"{"background":"opaque","user":"u-1","output_format":"png"}"#,
            "{}",
            FLUX1,
            false,
        )
        .expect("delegated"),
        object(r#"{"background":"opaque","user":"u-1","output_format":"png"}"#)
    );
    assert_eq!(
        mapped(
            r#"{"quality":"high"}"#,
            r#"{"quality":"low","custom":true}"#,
            FLUX1,
            true,
        )
        .expect("kept"),
        object(r#"{"quality":"low","custom":true}"#)
    );
    assert_eq!(
        mapped(r#"{"style":"vivid"}"#, "{}", FLUX1, true).expect("dropped"),
        object("{}")
    );
}

fn gpt_unsupported() {
    let width = mapped(r#"{"width":10}"#, "{}", FLUX1, false).expect_err("unsupported");
    assert_eq!(
        width,
        FoundryImageError::GptParams(ImageParamError::Unsupported {
            parameter: String::from("width"),
            model: String::from(FLUX1),
            supported: &GPT_IMAGE_PARAMS,
        })
    );
    assert_eq!(width.status_code(), None);
    assert_eq!(width.llm_provider(), None);
    let message = width.to_string();
    assert!(message.starts_with("parameter width is not supported for model "));
    assert!(message.contains(FLUX1));
    assert_eq!(
        mapped(r#"{"steps":4}"#, "{}", FLUX1, false).expect_err("unsupported"),
        FoundryImageError::GptParams(ImageParamError::Unsupported {
            parameter: String::from("steps"),
            model: String::from(FLUX1),
            supported: &GPT_IMAGE_PARAMS,
        })
    );
    assert_eq!(
        mapped(
            r#"{"style":"vivid","response_format":"b64_json"}"#,
            "{}",
            FLUX1,
            false,
        )
        .expect_err("unsupported"),
        FoundryImageError::GptParams(ImageParamError::Unsupported {
            parameter: String::from("style"),
            model: String::from(FLUX1),
            supported: &GPT_IMAGE_PARAMS,
        })
    );
}
