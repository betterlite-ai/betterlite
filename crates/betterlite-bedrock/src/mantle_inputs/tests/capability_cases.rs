use crate::mantle_inputs::capability::{base_segment, supports_responses};
use crate::model_routes::ParamValue;

use super::support::{cost, empty_cost, entry, list, text};

const MODEL: &str = "openai.gpt-5";
const MODEL_KEY: &str = "bedrock_mantle/openai.gpt-5";

pub(super) fn mantle_model_capabilities() {
    endpoint_cases();
    mode_cases();
    segment_cases();
    absent_model_cases();
    segment_flag_cases();
}

fn endpoint_cases() {
    let listed = cost(&[(
        MODEL_KEY,
        entry(&[
            (
                "supported_endpoints",
                list(&["/v1/chat/completions", "/v1/responses"]),
            ),
            ("mode", text("chat")),
        ]),
    )]);
    assert!(supports_responses(Some(MODEL), &listed));
    assert!(supports_responses(Some("eu-west-1/openai.gpt-5"), &listed));
    assert!(!supports_responses(None, &listed));
    assert!(!supports_responses(Some(""), &listed));
    assert!(!supports_responses(Some("openai.gpt-oss-120b"), &listed));
    assert!(!supports_responses(Some("google.gemma-4"), &listed));

    let chat_only = cost(&[(
        MODEL_KEY,
        entry(&[("supported_endpoints", list(&["/v1/chat/completions"]))]),
    )]);
    assert!(!supports_responses(Some(MODEL), &chat_only));
    let empty_list = cost(&[(MODEL_KEY, entry(&[("supported_endpoints", list(&[]))]))]);
    assert!(!supports_responses(Some(MODEL), &empty_list));
    let null_list = cost(&[(
        MODEL_KEY,
        entry(&[("supported_endpoints", ParamValue::Null)]),
    )]);
    assert!(!supports_responses(Some(MODEL), &null_list));

    // Python's `in` over a scalar string is a substring test
    let substring = cost(&[(
        MODEL_KEY,
        entry(&[("supported_endpoints", text("x/v1/responses/y"))]),
    )]);
    assert!(supports_responses(Some(MODEL), &substring));
    let substring_missing = cost(&[(
        MODEL_KEY,
        entry(&[("supported_endpoints", text("/v1/chat/completions"))]),
    )]);
    assert!(!supports_responses(Some(MODEL), &substring_missing));
}

fn mode_cases() {
    let mode = cost(&[(MODEL_KEY, entry(&[("mode", text("responses"))]))]);
    assert!(supports_responses(Some(MODEL), &mode));
    let upper_mode = cost(&[(MODEL_KEY, entry(&[("mode", text("Responses"))]))]);
    assert!(!supports_responses(Some(MODEL), &upper_mode));
    let chat_mode = cost(&[(MODEL_KEY, entry(&[("mode", text("chat"))]))]);
    assert!(!supports_responses(Some(MODEL), &chat_mode));
    assert!(!supports_responses(Some(MODEL), &empty_cost()));
}

fn segment_cases() {
    let openai_path = cost(&[(
        MODEL_KEY,
        entry(&[("use_openai_responses_path", ParamValue::Bool(true))]),
    )]);
    assert_eq!(base_segment(Some(MODEL), &openai_path), "openai/v1");
    assert_eq!(
        base_segment(Some("eu-west-1/openai.gpt-5"), &openai_path),
        "openai/v1"
    );
    let double_prefix = cost(&[(
        "bedrock_mantle/bedrock_mantle/openai.gpt-5",
        entry(&[("use_openai_responses_path", ParamValue::Bool(true))]),
    )]);
    assert_eq!(
        base_segment(Some("bedrock_mantle/openai.gpt-5"), &double_prefix),
        "openai/v1"
    );
}

fn absent_model_cases() {
    let configured = cost(&[(
        MODEL_KEY,
        entry(&[("use_openai_responses_path", ParamValue::Bool(true))]),
    )]);
    assert_eq!(base_segment(None, &configured), "v1");
    assert_eq!(base_segment(Some(""), &configured), "v1");
    assert_eq!(base_segment(Some(MODEL), &empty_cost()), "v1");
}

fn segment_flag_cases() {
    // `is True` in LiteLLM, so every other JSON spelling of truth keeps the plain v1 base
    let flags: [(&str, ParamValue); 4] = [
        ("openai.gpt-5-pro", ParamValue::Bool(false)),
        (
            "openai.gpt-5-nano",
            ParamValue::Number(serde_json::Number::from(1_u64)),
        ),
        ("openai.gpt-4.1", text("true")),
        ("openai.gpt-5-lite", ParamValue::Null),
    ];
    for (model, flag) in flags {
        let configured = cost(&[(
            format!("bedrock_mantle/{model}").as_str(),
            entry(&[("use_openai_responses_path", flag)]),
        )]);
        assert_eq!(base_segment(Some(model), &configured), "v1", "{model}");
    }
}
