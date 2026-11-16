use super::support::{mapped, object};
use crate::image_generation::error::FoundryImageError;
use crate::image_generation::flux::FLUX2_OPENAI_PARAMS;

const FLUX2: &str = "azure_ai/FLUX.2-pro";
const SQUARE: &str = r#"{"width":1024,"height":768}"#;
const SQUARE_512: &str = r#"{"width":512,"height":512}"#;
const KEPT_OPTIONAL: &str = r#"{"width":1,"height":2,"quality":"low","custom":true}"#;
const KEPT_EXPECTED: &str = r#"{"width":512,"height":512,"quality":"low","custom":true}"#;
const WIDTH_AFTER_SIZE: &str = r#"{"width":300,"height":512}"#;
const WIDTH_ONE: &str = r#"{"width":1}"#;

pub(super) fn flux_native_fields() {
    size_auto();
    size_dimensions();
    size_overwrites();
    unsupported_parameters();
    invalid_sizes();
    non_text_sizes();
}

fn expect_size(non_default: &str, optional: &str, expected: &str) {
    assert_eq!(
        mapped(non_default, optional, FLUX2, false).expect("mapped"),
        object(expected)
    );
}

fn size_auto() {
    expect_size(r#"{"size":"auto"}"#, "{}", "{}");
    expect_size(r#"{"size":"AUTO"}"#, "{}", "{}");
    expect_size(r#"{"size":"Auto","width":1}"#, "{}", WIDTH_ONE);
    expect_size(r#"{"size":"AUTO"}"#, WIDTH_ONE, WIDTH_ONE);
}

fn size_dimensions() {
    expect_size(r#"{"size":"1024x768"}"#, "{}", SQUARE);
    expect_size(r#"{"size":"1024X768"}"#, "{}", SQUARE);
    expect_size(r#"{"size":" 1024 x 768 "}"#, "{}", SQUARE);
}

fn size_overwrites() {
    expect_size(r#"{"size":"512x512"}"#, KEPT_OPTIONAL, KEPT_EXPECTED);
    expect_size(r#"{"width":300,"size":"512x512"}"#, "{}", SQUARE_512);
    expect_size(r#"{"size":"512x512","width":300}"#, "{}", WIDTH_AFTER_SIZE);
    expect_size(
        r#"{"quality":"high"}"#,
        r#"{"quality":"low"}"#,
        r#"{"quality":"low"}"#,
    );
}

fn unsupported_parameters() {
    let unsupported = mapped(
        r#"{"style":"vivid","response_format":"b64_json"}"#,
        "{}",
        FLUX2,
        false,
    )
    .expect_err("unsupported parameters");
    assert_eq!(
        unsupported,
        FoundryImageError::UnsupportedParams {
            model: String::from(FLUX2),
            names: vec![String::from("style"), String::from("response_format")],
            supported: &FLUX2_OPENAI_PARAMS,
        }
    );
    assert_eq!(unsupported.status_code(), Some(400));
    assert_eq!(unsupported.llm_provider(), Some("azure_ai"));
    assert_eq!(
        unsupported.to_string(),
        format!(
            "Parameters [\"style\", \"response_format\"] are not supported for model {FLUX2}. Supported parameters are {FLUX2_OPENAI_PARAMS:?}. Set drop_params=True to drop unsupported parameters."
        )
    );
    assert_eq!(
        mapped(
            r#"{"style":"vivid","response_format":"b64_json"}"#,
            "{}",
            FLUX2,
            true,
        )
        .expect("dropped"),
        object("{}")
    );
}

fn invalid_sizes() {
    assert_eq!(
        mapped(r#"{"style":"vivid","size":"bad"}"#, "{}", FLUX2, false)
            .expect_err("unsupported before the size"),
        FoundryImageError::UnsupportedParams {
            model: String::from(FLUX2),
            names: vec![String::from("style")],
            supported: &FLUX2_OPENAI_PARAMS,
        }
    );
    let invalid = mapped(r#"{"style":"vivid","size":"bad"}"#, "{}", FLUX2, true)
        .expect_err("an invalid size");
    assert_eq!(
        invalid,
        FoundryImageError::InvalidSize {
            size: String::from("bad"),
        }
    );
    assert_eq!(invalid.status_code(), Some(400));
    assert_eq!(invalid.llm_provider(), Some("azure_ai"));
    assert_eq!(
        invalid.to_string(),
        "Invalid size format 'bad'. Expected 'WxH', for example '1024x1024'."
    );
    malformed_sizes();
}

fn malformed_sizes() {
    for size in [
        "1024",
        "1024x",
        "x768",
        "1024x768x1",
        "axb",
        "",
        "auto ",
        "auto x",
    ] {
        let text = format!(r#"{{"size":{size:?}}}"#);
        let error = mapped(&text, "{}", FLUX2, false).expect_err("an invalid size");
        assert_eq!(
            error,
            FoundryImageError::InvalidSize {
                size: String::from(size),
            }
        );
    }
}

fn non_text_sizes() {
    assert_eq!(
        mapped(r#"{"size":1024}"#, "{}", FLUX2, false).expect_err("an invalid size"),
        FoundryImageError::InvalidSize {
            size: String::from("1024"),
        }
    );
    assert_eq!(
        mapped(r#"{"size":null}"#, "{}", FLUX2, false).expect_err("an invalid size"),
        FoundryImageError::InvalidSize {
            size: String::from("null"),
        }
    );
}
