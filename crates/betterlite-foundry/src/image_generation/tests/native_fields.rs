use super::support::{failure, mapped, object};
use crate::image_generation::error::FoundryImageError;
use crate::image_generation::flux::FLUX2_OPENAI_PARAMS;

const FLUX2: &str = "azure_ai/FLUX.2-pro";

pub(super) fn flux_native_fields() {
    size_auto();
    size_dimensions();
    size_overwrites();
    unsupported_parameters();
    invalid_sizes();
    non_text_sizes();
}

fn size_auto() {
    assert_eq!(
        mapped(r#"{"size":"auto"}"#, "{}", FLUX2, false).expect("mapped"),
        object("{}")
    );
    assert_eq!(
        mapped(r#"{"size":"AUTO"}"#, "{}", FLUX2, false).expect("mapped"),
        object("{}")
    );
    assert_eq!(
        mapped(r#"{"size":"Auto","width":1}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"width":1}"#)
    );
    assert_eq!(
        mapped(r#"{"size":"AUTO"}"#, r#"{"width":1}"#, FLUX2, false).expect("mapped"),
        object(r#"{"width":1}"#)
    );
}

fn size_dimensions() {
    assert_eq!(
        mapped(r#"{"size":"1024x768"}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"width":1024,"height":768}"#)
    );
    assert_eq!(
        mapped(r#"{"size":"1024X768"}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"width":1024,"height":768}"#)
    );
    assert_eq!(
        mapped(r#"{"size":" 1024 x 768 "}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"width":1024,"height":768}"#)
    );
}

fn size_overwrites() {
    assert_eq!(
        mapped(
            r#"{"size":"512x512"}"#,
            r#"{"width":1,"height":2,"quality":"low","custom":true}"#,
            FLUX2,
            false,
        )
        .expect("mapped"),
        object(r#"{"width":512,"height":512,"quality":"low","custom":true}"#)
    );
    assert_eq!(
        mapped(r#"{"width":300,"size":"512x512"}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"width":512,"height":512}"#)
    );
    assert_eq!(
        mapped(r#"{"size":"512x512","width":300}"#, "{}", FLUX2, false).expect("mapped"),
        object(r#"{"width":300,"height":512}"#)
    );
    assert_eq!(
        mapped(
            r#"{"quality":"high"}"#,
            r#"{"quality":"low"}"#,
            FLUX2,
            false
        )
        .expect("kept"),
        object(r#"{"quality":"low"}"#)
    );
}

fn unsupported_parameters() {
    let unsupported = failure(
        r#"{"style":"vivid","response_format":"b64_json"}"#,
        "{}",
        FLUX2,
        false,
    );
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
        failure(r#"{"style":"vivid","size":"bad"}"#, "{}", FLUX2, false),
        FoundryImageError::UnsupportedParams {
            model: String::from(FLUX2),
            names: vec![String::from("style")],
            supported: &FLUX2_OPENAI_PARAMS,
        }
    );
    let invalid = failure(r#"{"style":"vivid","size":"bad"}"#, "{}", FLUX2, true);
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
        let error = failure(&text, "{}", FLUX2, false);
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
        failure(r#"{"size":1024}"#, "{}", FLUX2, false),
        FoundryImageError::InvalidSize {
            size: String::from("1024"),
        }
    );
    assert_eq!(
        failure(r#"{"size":null}"#, "{}", FLUX2, false),
        FoundryImageError::InvalidSize {
            size: String::from("null"),
        }
    );
}
