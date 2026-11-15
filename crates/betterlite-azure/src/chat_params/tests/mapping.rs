use betterlite_openai::reasoning_models::DropParams;

use super::support::{mapped, object};
use crate::chat_params::error::ChatParamsError;

const NONE_EFFORT: &str = r#"{"reasoning_effort":"none"}"#;
const DICT_NONE_EFFORT: &str = r#"{"reasoning_effort":{"effort":"none","summary":"auto"}}"#;

pub(super) fn azure_effort_none_drop() {
    let rejected = mapped(
        "gpt5_series/gpt-5.3",
        NONE_EFFORT,
        "{}",
        DropParams::default(),
    )
    .unwrap_err();
    assert!(matches!(rejected, ChatParamsError::UnsupportedNone));
    assert_eq!(rejected.status_code(), Some(400));
    let diagnostic = rejected.to_string();
    assert!(
        diagnostic
            .starts_with("Azure OpenAI does not support reasoning_effort='none' for this model.")
    );
    assert!(diagnostic.contains("Supported values are: 'low', 'medium', and 'high'."));
    assert!(diagnostic.contains("litellm.drop_params=True"));
    assert!(diagnostic.contains("issues/16704"));
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.3",
            NONE_EFFORT,
            "{}",
            DropParams {
                global: false,
                request: true
            }
        )
        .unwrap(),
        object("{}")
    );
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.3",
            NONE_EFFORT,
            "{}",
            DropParams {
                global: true,
                request: false
            }
        )
        .unwrap(),
        object("{}")
    );
    let from_dict = mapped(
        "gpt5_series/gpt-5.3",
        DICT_NONE_EFFORT,
        "{}",
        DropParams::default(),
    )
    .unwrap_err();
    assert!(matches!(from_dict, ChatParamsError::UnsupportedNone));
    assert_eq!(from_dict.status_code(), Some(400));
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.3",
            DICT_NONE_EFFORT,
            "{}",
            DropParams {
                global: false,
                request: true
            }
        )
        .unwrap(),
        object("{}")
    );
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.1",
            NONE_EFFORT,
            r#"{"temperature":0.2}"#,
            DropParams::default()
        )
        .unwrap(),
        object(r#"{"reasoning_effort":"none","temperature":0.2}"#)
    );
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.3",
            NONE_EFFORT,
            r#"{"reasoning_effort":"medium"}"#,
            DropParams {
                global: true,
                request: false
            }
        )
        .unwrap(),
        object(r#"{"reasoning_effort":"medium"}"#)
    );
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.3",
            r#"{"reasoning_effort":"medium"}"#,
            "{}",
            DropParams::default()
        )
        .unwrap(),
        object(r#"{"reasoning_effort":"medium"}"#)
    );
    let from_optional = mapped(
        "gpt5_series/gpt-5.3",
        r#"{"reasoning_effort":null}"#,
        NONE_EFFORT,
        DropParams::default(),
    )
    .unwrap_err();
    assert!(matches!(from_optional, ChatParamsError::UnsupportedNone));
    assert_eq!(from_optional.status_code(), Some(400));
    let disabled = mapped(
        "gpt5_series/gpt-5.3",
        r#"{"reasoning_effort":"low"}"#,
        "{}",
        DropParams::default(),
    )
    .unwrap_err();
    assert!(matches!(disabled, ChatParamsError::Reasoning(_)));
    assert_eq!(disabled.status_code(), Some(400));
    assert_eq!(
        disabled.to_string(),
        "reasoning_effort=low is not supported for this model"
    );
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.1",
            NONE_EFFORT,
            "{}",
            DropParams::default()
        )
        .unwrap(),
        object(NONE_EFFORT)
    );
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.1",
            r#"{"logprobs":true}"#,
            "{}",
            DropParams::default()
        )
        .unwrap(),
        object("{}")
    );
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.1",
            r#"{"logprobs":true}"#,
            r#"{"logprobs":false}"#,
            DropParams::default()
        )
        .unwrap(),
        object(r#"{"logprobs":false}"#)
    );
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.1",
            r#"{"logprobs":true,"reasoning_effort":"none"}"#,
            "{}",
            DropParams::default()
        )
        .unwrap(),
        object(NONE_EFFORT)
    );
    assert_eq!(
        mapped(
            "gpt5_series/gpt-5.2-pro",
            r#"{"logprobs":true}"#,
            "{}",
            DropParams::default()
        )
        .unwrap(),
        object(r#"{"logprobs":true}"#)
    );
    assert_eq!(
        mapped(
            "my-gpt5-deployment",
            r#"{"tool_choice":"auto"}"#,
            "{}",
            DropParams::default()
        )
        .unwrap(),
        object(r#"{"tool_choice":"auto"}"#)
    );
}
