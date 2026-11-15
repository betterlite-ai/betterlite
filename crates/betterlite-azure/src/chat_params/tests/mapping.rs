use betterlite_openai::reasoning_models::DropParams;

use super::support::{mapped, object};
use crate::chat_params::error::ChatParamsError;

const NONE_EFFORT: &str = r#"{"reasoning_effort":"none"}"#;
const DICT_NONE_EFFORT: &str = r#"{"reasoning_effort":{"effort":"none","summary":"auto"}}"#;
const MEDIUM_EFFORT: &str = r#"{"reasoning_effort":"medium"}"#;
const LOGPROBS: &str = r#"{"logprobs":true}"#;
const NO_DROP: DropParams = DropParams {
    global: false,
    request: false,
};
const REQUEST_DROP: DropParams = DropParams {
    global: false,
    request: true,
};
const GLOBAL_DROP: DropParams = DropParams {
    global: true,
    request: false,
};

// (model, non-default params, optional params, drop_params, mapped optional params)
const MAPPED: [(&str, &str, &str, DropParams, &str); 12] = [
    ("gpt5_series/gpt-5.3", NONE_EFFORT, "{}", REQUEST_DROP, "{}"),
    ("gpt5_series/gpt-5.3", NONE_EFFORT, "{}", GLOBAL_DROP, "{}"),
    (
        "gpt5_series/gpt-5.3",
        DICT_NONE_EFFORT,
        "{}",
        REQUEST_DROP,
        "{}",
    ),
    (
        "gpt5_series/gpt-5.1",
        NONE_EFFORT,
        r#"{"temperature":0.2}"#,
        NO_DROP,
        r#"{"reasoning_effort":"none","temperature":0.2}"#,
    ),
    (
        "gpt5_series/gpt-5.3",
        NONE_EFFORT,
        MEDIUM_EFFORT,
        GLOBAL_DROP,
        MEDIUM_EFFORT,
    ),
    (
        "gpt5_series/gpt-5.3",
        MEDIUM_EFFORT,
        "{}",
        NO_DROP,
        MEDIUM_EFFORT,
    ),
    (
        "gpt5_series/gpt-5.1",
        NONE_EFFORT,
        "{}",
        NO_DROP,
        NONE_EFFORT,
    ),
    ("gpt5_series/gpt-5.1", LOGPROBS, "{}", NO_DROP, "{}"),
    (
        "gpt5_series/gpt-5.1",
        LOGPROBS,
        r#"{"logprobs":false}"#,
        NO_DROP,
        r#"{"logprobs":false}"#,
    ),
    (
        "gpt5_series/gpt-5.1",
        r#"{"logprobs":true,"reasoning_effort":"none"}"#,
        "{}",
        NO_DROP,
        NONE_EFFORT,
    ),
    ("gpt5_series/gpt-5.2-pro", LOGPROBS, "{}", NO_DROP, LOGPROBS),
    (
        "my-gpt5-deployment",
        r#"{"tool_choice":"auto"}"#,
        "{}",
        NO_DROP,
        r#"{"tool_choice":"auto"}"#,
    ),
];

// (model, non-default params, optional params) that raise the Azure none diagnostic without drop_params
const UNSUPPORTED_NONE: [(&str, &str, &str); 3] = [
    ("gpt5_series/gpt-5.3", NONE_EFFORT, "{}"),
    ("gpt5_series/gpt-5.3", DICT_NONE_EFFORT, "{}"),
    (
        "gpt5_series/gpt-5.3",
        r#"{"reasoning_effort":null}"#,
        NONE_EFFORT,
    ),
];

pub(super) fn azure_effort_none_drop() {
    unsupported_none_diagnostic();
    unsupported_none_rejections();
    mapped_rows();
    disabled_low_effort();
}

fn unsupported_none_diagnostic() {
    let rejected = mapped("gpt5_series/gpt-5.3", NONE_EFFORT, "{}", NO_DROP).unwrap_err();
    let diagnostic = rejected.to_string();
    assert!(
        diagnostic
            .starts_with("Azure OpenAI does not support reasoning_effort='none' for this model.")
    );
    assert!(diagnostic.contains("Supported values are: 'low', 'medium', and 'high'."));
    assert!(diagnostic.contains("litellm.drop_params=True"));
    assert!(diagnostic.contains("issues/16704"));
}

fn unsupported_none_rejections() {
    for (model, requested, optional) in UNSUPPORTED_NONE {
        let rejected = mapped(model, requested, optional, NO_DROP).unwrap_err();
        assert!(matches!(rejected, ChatParamsError::UnsupportedNone));
        assert_eq!(rejected.status_code(), Some(400));
    }
}

fn mapped_rows() {
    for (model, requested, optional, drop, expected) in MAPPED {
        assert_eq!(
            mapped(model, requested, optional, drop).unwrap(),
            object(expected)
        );
    }
}

fn disabled_low_effort() {
    let disabled = mapped(
        "gpt5_series/gpt-5.3",
        r#"{"reasoning_effort":"low"}"#,
        "{}",
        NO_DROP,
    )
    .unwrap_err();
    assert!(matches!(disabled, ChatParamsError::Reasoning(_)));
    assert_eq!(disabled.status_code(), Some(400));
    assert_eq!(
        disabled.to_string(),
        "reasoning_effort=low is not supported for this model"
    );
}
