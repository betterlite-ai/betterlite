use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;

use crate::azure_chat_base_policy::SUPPORTED_OPENAI_PARAMS;
use crate::azure_chat_base_policy::error::PolicyError;
use crate::azure_chat_base_policy::mapping::{
    format_supported_for_version, requires_max_completion_tokens, supports_response_format,
};

use super::support::{
    DropMode, REQUIRED_UPGRADE, TOOL_UPGRADE, blocked, mapped, object, policy,
    required_tool_choice_error, tool_choice_error,
};

const EMPTY: &str = "{}";
const TOOL_AUTO: &str = r#"{"tool_choice":"auto"}"#;
const TOOL_REQUIRED: &str = r#"{"tool_choice":"required"}"#;
const FORMAT_REQUEST: &str = r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"}}}}"#;
const EMULATED_FORMAT: &str = r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#;
const TOKENS: &str = r#"{"max_tokens":100}"#;
const TOKENS_TEMP: &str = r#"{"max_tokens":100,"temperature":0.2}"#;
const COMPLETION_TEMP: &str = r#"{"max_completion_tokens":100,"temperature":0.2}"#;
const COMPLETION_SEVEN: &str = r#"{"max_completion_tokens":7}"#;
const EVERY_PARAM: &str = r#"{"temperature":0.5,"n":2,"stream":true,"stream_options":{"include_usage":true},"stop":"","max_completion_tokens":4,"presence_penalty":0,"frequency_penalty":0,"logit_bias":{},"user":"u","function_call":"auto","functions":[],"top_p":0.9,"logprobs":true,"top_logprobs":2,"response_format":"json_object","seed":1,"extra_headers":{"x":"y"},"parallel_tool_calls":false,"prediction":{},"modalities":["text"],"audio":{},"web_search_options":{},"prompt_cache_key":"p","store":true,"unknown_param":1}"#;
const KEPT_PARAM: &str = r#"{"temperature":0.5,"n":2,"stream":true,"stream_options":{"include_usage":true},"stop":"","max_completion_tokens":4,"presence_penalty":0,"frequency_penalty":0,"logit_bias":{},"user":"u","function_call":"auto","functions":[],"top_p":0.9,"logprobs":true,"top_logprobs":2,"response_format":"json_object","seed":1,"extra_headers":{"x":"y"},"parallel_tool_calls":false,"prediction":{},"modalities":["text"],"audio":{},"web_search_options":{},"prompt_cache_key":"p","store":true}"#;
const TOOLS_A: &str = r#"{"tools":[{"type":"function","function":{"name":"a"}}]}"#;
const TOOLS_B: &str = r#"{"tools":[{"type":"function","function":{"name":"b"}}]}"#;
const TOOLS_AB: &str = r#"{"tools":[{"type":"function","function":{"name":"a"}},{"type":"function","function":{"name":"b"}}]}"#;
const TOOLS_X: &str = r#"{"tools":"x"}"#;
const TOOLS_NULL: &str = r#"{"tools":null}"#;
const TOOLS_DICT: &str = r#"{"tools":{"a":1}}"#;
const TOOLS_EMPTY: &str = r#"{"tools":[]}"#;

fn assert_mapped(non_default: &str, initial: &str, model: &str, api_version: &str, expected: &str) {
    assert_eq!(
        mapped(
            &object(non_default),
            initial,
            &policy(model, api_version, DropMode::Off)
        ),
        object(expected)
    );
}

fn assert_unsupported(error: &PolicyError, message: &str) {
    assert_eq!(error.to_string(), message);
    assert_eq!(error.status_code(), Some(400));
    let metadata = error.classify().unwrap().metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::MISSING);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.code, "400");
}

const SUPPORTED_MODELS: [(&str, bool); 9] = [
    ("gpt-4o", true),
    ("gpt-4.1", true),
    ("gpt-4-1", true),
    ("azure/gpt-4-1", true),
    ("gpt-3x5", true),
    ("gpt-3.5-turbo", false),
    ("gpt-3-5-turbo", false),
    ("gpt-35-turbo", false),
    ("azure/gpt-3-5", false),
];

const SUPPORTED_VERSIONS: [(&str, &str, bool); 10] = [
    ("2024", "07", false),
    ("2024", "08", true),
    ("2024", "09", true),
    ("2025", "01", true),
    ("2023", "12", false),
    (" 2024 ", " 8 ", true),
    ("2_024", "8", true),
    ("+2024", "+08", true),
    ("-1", "8", false),
    ("2024", "-1", false),
];

const FORMAT_CASES: [(&str, &str, &str, &str, &str); 8] = [
    (
        FORMAT_REQUEST,
        EMPTY,
        "gpt-4o",
        "2024-08-01",
        FORMAT_REQUEST,
    ),
    (FORMAT_REQUEST, EMPTY, "gpt-4o", "2024-08", FORMAT_REQUEST),
    (FORMAT_REQUEST, EMPTY, "gpt-4o", "", FORMAT_REQUEST),
    (
        FORMAT_REQUEST,
        EMPTY,
        "gpt-3.5-turbo",
        "2024-08-01",
        EMULATED_FORMAT,
    ),
    (
        FORMAT_REQUEST,
        EMPTY,
        "gpt-4o",
        "2024-07-01",
        EMULATED_FORMAT,
    ),
    (
        FORMAT_REQUEST,
        EMPTY,
        "gpt-35-turbo",
        "2025-01-01",
        EMULATED_FORMAT,
    ),
    (
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{}}}}"#,
        EMPTY,
        "gpt-3.5-turbo",
        "2024-08-01",
        EMPTY,
    ),
    (
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{}}}}"#,
        EMPTY,
        "gpt-4o",
        "2024-08-01",
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{}}}}"#,
    ),
];

const REQUIRE_MODELS: [(&str, bool); 8] = [
    ("gpt-5", true),
    ("azure/gpt-5-chat", true),
    ("gpt-6-mini", true),
    ("gpt5_series/model", true),
    ("GPT-5", false),
    ("gpt-4o", false),
    ("o1", false),
    ("gpt-4-5", false),
];

const COMPLETION_CASES: [(&str, &str, &str, &str); 10] = [
    (TOKENS, EMPTY, "gpt-5", r#"{"max_completion_tokens":100}"#),
    (
        TOKENS,
        EMPTY,
        "azure/gpt-5-chat",
        r#"{"max_completion_tokens":100}"#,
    ),
    (TOKENS, EMPTY, "gpt-4o", TOKENS),
    (TOKENS_TEMP, EMPTY, "gpt-5", COMPLETION_TEMP),
    (COMPLETION_SEVEN, EMPTY, "gpt-5", COMPLETION_SEVEN),
    (
        TOKENS,
        r#"{"max_completion_tokens":7}"#,
        "gpt-5",
        COMPLETION_SEVEN,
    ),
    (
        r#"{"max_tokens":100,"max_completion_tokens":7}"#,
        EMPTY,
        "gpt-5",
        COMPLETION_SEVEN,
    ),
    (
        r#"{"max_completion_tokens":7,"max_tokens":100}"#,
        EMPTY,
        "gpt-5",
        COMPLETION_SEVEN,
    ),
    (
        r#"{"max_tokens":100,"temperature":0.1}"#,
        r#"{"max_tokens":5}"#,
        "gpt-5",
        r#"{"max_tokens":5,"max_completion_tokens":100,"temperature":0.1}"#,
    ),
    (TOKENS, EMPTY, "GPT-5", TOKENS),
];

const PARAM_CASES: [(&str, &str); 3] = [
    (EVERY_PARAM, KEPT_PARAM),
    (r#"{"unknown_param":1}"#, EMPTY),
    (r#"{"temperature":null}"#, r#"{"temperature":null}"#),
];

const REJECTED_DATES: [&str; 4] = ["2023-11-30", "2023-12-00", "2022-12-31", "0000-01-01"];

const ALLOWED_DATES: [&str; 11] = [
    "2023-12-01",
    "2023-12-02",
    "2023-12-1",
    "2023-2-01",
    "2023-13-01",
    "2024-01-01",
    "2024-12-00",
    "2023-11",
    "",
    "abc",
    "not-a-date",
];

const REQUIRED_REJECTED: [&str; 4] = ["2024-05-31", "2024-04-01", "2024-02-01", "2024-05-01"];

const REQUIRED_ALLOWED: [&str; 4] = ["2024-5-01", "2024-06-01", "2023-12-01", "2023-12-1"];

const FORWARDED_CHOICES: [&str; 4] = [
    r#"{"tool_choice":""}"#,
    r#"{"tool_choice":false}"#,
    r#"{"tool_choice":"none"}"#,
    TOOL_AUTO,
];

pub(super) fn api_version_response_format() {
    response_format_models();
    response_format_versions();
    response_format_invalid_versions();
    response_format_mapping();
    response_format_invalid_dates();
}

fn response_format_models() {
    for (model, expected) in SUPPORTED_MODELS {
        assert_eq!(supports_response_format(model), expected, "{model}");
    }
}

fn response_format_versions() {
    for (year, month, expected) in SUPPORTED_VERSIONS {
        assert_eq!(
            format_supported_for_version(year, month).unwrap(),
            expected,
            "{year}-{month}"
        );
    }
}

fn assert_version_error(year: &str, month: &str, field: &'static str) {
    let error = format_supported_for_version(year, month).unwrap_err();
    assert!(
        matches!(error, PolicyError::ApiVersionNumber { field: actual, .. } if actual == field)
    );
}

fn response_format_invalid_versions() {
    for (year, month, field) in [
        ("2024", "x", "month"),
        ("x", "8", "year"),
        ("", "8", "year"),
        ("_12", "8", "year"),
        ("1__2", "8", "year"),
        ("1_", "8", "year"),
        ("2024", "", "month"),
    ] {
        assert_version_error(year, month, field);
    }
}

fn response_format_mapping() {
    for (non_default, initial, model, api_version, expected) in FORMAT_CASES {
        assert_mapped(non_default, initial, model, api_version, expected);
    }
}

fn assert_blocked_number(api_version: &str, field: &'static str) {
    let error = blocked(
        &object(FORMAT_REQUEST),
        EMPTY,
        &policy("gpt-4o", api_version, DropMode::Off),
    );
    assert!(
        matches!(error, PolicyError::ApiVersionNumber { field: actual, .. } if actual == field)
    );
}

fn response_format_invalid_dates() {
    assert_blocked_number("abcd-08-01", "year");
    assert_blocked_number("2024-ab-01", "month");
}

pub(super) fn max_completion_tokens() {
    max_completion_predicates();
    max_completion_mapping();
}

fn max_completion_predicates() {
    for (model, expected) in REQUIRE_MODELS {
        assert_eq!(requires_max_completion_tokens(model), expected, "{model}");
    }
}

fn max_completion_mapping() {
    for (non_default, initial, model, expected) in COMPLETION_CASES {
        assert_mapped(non_default, initial, model, "", expected);
    }
}

pub(super) fn unsupported_param() {
    supported_param_list();
    supported_param_mapping();
}

fn supported_param_list() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "temperature",
            "n",
            "stream",
            "stream_options",
            "stop",
            "max_tokens",
            "max_completion_tokens",
            "tools",
            "tool_choice",
            "presence_penalty",
            "frequency_penalty",
            "logit_bias",
            "user",
            "function_call",
            "functions",
            "tools",
            "tool_choice",
            "top_p",
            "logprobs",
            "top_logprobs",
            "response_format",
            "seed",
            "extra_headers",
            "parallel_tool_calls",
            "prediction",
            "modalities",
            "audio",
            "web_search_options",
            "prompt_cache_key",
            "store",
        ]
    );
}

fn supported_param_mapping() {
    for (non_default, expected) in PARAM_CASES {
        assert_mapped(non_default, EMPTY, "gpt-4o", "", expected);
    }
}

pub(super) fn azure_tool_choice_date_gate() {
    date_gate_message();
    date_gate_rejections();
    date_gate_allowed();
    date_gate_dropped();
}

fn date_gate_message() {
    let error = tool_choice_error("2022-01-01");
    assert_unsupported(&error, TOOL_UPGRADE);
}

fn date_gate_rejections() {
    for api_version in REJECTED_DATES {
        let error = tool_choice_error(api_version);
        assert!(
            matches!(error, PolicyError::ToolChoiceUnsupported { .. }),
            "{api_version}"
        );
        assert_eq!(error.status_code(), Some(400));
    }
    let null_choice = blocked(
        &object(r#"{"tool_choice":null}"#),
        EMPTY,
        &policy("gpt-4o", "2023-11-30", DropMode::Off),
    );
    assert_eq!(null_choice.status_code(), Some(400));
}

fn date_gate_allowed() {
    for api_version in ALLOWED_DATES {
        assert_mapped(TOOL_AUTO, EMPTY, "gpt-4o", api_version, TOOL_AUTO);
    }
}

fn date_gate_dropped() {
    let per_call = policy("gpt-4o", "2023-11-30", DropMode::PerCall);
    assert_eq!(mapped(&object(TOOL_AUTO), EMPTY, &per_call), object(EMPTY));
    let global = policy("gpt-4o", "2023-11-30", DropMode::Global);
    assert_eq!(mapped(&object(TOOL_AUTO), EMPTY, &global), object(EMPTY));
    let allowed = policy("gpt-4o", "2023-12-01", DropMode::PerCall);
    assert_eq!(
        mapped(&object(TOOL_AUTO), EMPTY, &allowed),
        object(TOOL_AUTO)
    );
}

pub(super) fn azure_required_tool_choice() {
    required_message();
    required_rejections();
    required_allowed();
    required_forwarded();
    required_dropped();
}

fn required_message() {
    let error = required_tool_choice_error("2024-05-01");
    assert_unsupported(&error, REQUIRED_UPGRADE);
}

fn required_rejections() {
    for api_version in REQUIRED_REJECTED {
        assert!(
            matches!(
                required_tool_choice_error(api_version),
                PolicyError::RequiredToolChoice { .. }
            ),
            "{api_version}"
        );
    }
    assert!(matches!(
        required_tool_choice_error("2022-01-01"),
        PolicyError::ToolChoiceUnsupported { .. }
    ));
}

fn required_allowed() {
    for api_version in REQUIRED_ALLOWED {
        assert_mapped(TOOL_REQUIRED, EMPTY, "gpt-4o", api_version, TOOL_REQUIRED);
    }
}

fn required_forwarded() {
    for non_default in FORWARDED_CHOICES {
        assert_mapped(non_default, EMPTY, "gpt-4o", "2024-05-01", non_default);
    }
}

fn required_dropped() {
    let per_call = policy("gpt-4o", "2024-05-01", DropMode::PerCall);
    assert_eq!(
        mapped(&object(TOOL_REQUIRED), EMPTY, &per_call),
        object(EMPTY)
    );
    let global = policy("gpt-4o", "2024-05-01", DropMode::Global);
    assert_eq!(
        mapped(&object(TOOL_REQUIRED), EMPTY, &global),
        object(EMPTY)
    );
}

pub(super) fn azure_parameter_shapes() {
    tools_extend();
    tools_malformed();
    tools_passthrough();
    format_tools();
    format_errors();
    format_native();
    format_malformed_tools();
    format_order();
}

fn tools_extend() {
    assert_mapped(TOOLS_B, TOOLS_A, "gpt-4o", "", TOOLS_AB);
}

fn tools_malformed() {
    for initial in [TOOLS_X, TOOLS_NULL] {
        let error = blocked(
            &object(TOOLS_B),
            initial,
            &policy("gpt-4o", "", DropMode::Off),
        );
        assert!(matches!(error, PolicyError::ToolsNotList));
    }
}

fn tools_passthrough() {
    assert_mapped(TOOLS_DICT, EMPTY, "gpt-4o", "", TOOLS_DICT);
    assert_mapped(TOOLS_DICT, TOOLS_EMPTY, "gpt-4o", "", TOOLS_DICT);
}

fn format_tools() {
    assert_mapped(
        FORMAT_REQUEST,
        TOOLS_A,
        "gpt-3.5-turbo",
        "2024-08-01",
        r#"{"tools":[{"type":"function","function":{"name":"a"}},{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#,
    );
    assert_mapped(
        r#"{"response_format":{"response_schema":{"type":"object","properties":{"a":{"type":"string"}}},"json_schema":{"name":"n","schema":{"type":"string"}}}}"#,
        EMPTY,
        "gpt-3.5-turbo",
        "2024-08-01",
        r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object","properties":{"a":{"type":"string"}}}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#,
    );
}

fn format_errors() {
    for non_default in [
        r#"{"response_format":{"json_schema":{"name":"n"}}}"#,
        r#"{"response_format":{"json_schema":"x"}}"#,
    ] {
        let error = blocked(
            &object(non_default),
            EMPTY,
            &policy("gpt-3.5-turbo", "2024-08-01", DropMode::Off),
        );
        assert!(matches!(error, PolicyError::ResponseFormat { .. }));
    }
}

fn assert_bad_request(error: &PolicyError) {
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(error.classify(), Some(ErrorClass::BadRequest(_))));
}

const MISSING_SCHEMA: &str = r#"{"response_format":{"json_schema":{"name":"n"}}}"#;
const NON_OBJECT_SCHEMA: &str = r#"{"response_format":{"json_schema":"x"}}"#;

pub(super) fn response_format_error_class() {
    let mapping = policy("gpt-4o", "", DropMode::Off);
    let error = blocked(&object(MISSING_SCHEMA), EMPTY, &mapping);
    assert!(matches!(error, PolicyError::ResponseFormat { .. }));
    assert_bad_request(&error);
}

pub(super) fn non_object_schema_error_class() {
    let mapping = policy("gpt-4o", "", DropMode::Off);
    let error = blocked(&object(NON_OBJECT_SCHEMA), EMPTY, &mapping);
    assert!(matches!(error, PolicyError::ResponseFormat { .. }));
    assert_bad_request(&error);
}

pub(super) fn tools_not_list_error_class() {
    let mapping = policy("gpt-4o", "", DropMode::Off);
    let error = blocked(&object(TOOLS_B), TOOLS_X, &mapping);
    assert!(matches!(error, PolicyError::ToolsNotList));
    assert_bad_request(&error);
}

const NATIVE_CASES: [(&str, &str, &str, &str, &str); 2] = [
    (
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{}}}}"#,
        EMPTY,
        "gpt-4o",
        "2024-08-01",
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{}}}}"#,
    ),
    (
        r#"{"response_format":"json_object"}"#,
        EMPTY,
        "gpt-3.5-turbo",
        "2024-08-01",
        r#"{"response_format":"json_object"}"#,
    ),
];

fn format_native() {
    for (non_default, initial, model, api_version, expected) in NATIVE_CASES {
        assert_mapped(non_default, initial, model, api_version, expected);
    }
}

const MALFORMED_NATIVE: &str = r#"{"tools":"x","response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"}}}}"#;

fn format_malformed_tools() {
    format_tools_not_list();
    format_native_malformed();
}

fn format_tools_not_list() {
    let mapping = policy("gpt-3.5-turbo", "2024-08-01", DropMode::Off);
    let error = blocked(&object(FORMAT_REQUEST), TOOLS_X, &mapping);
    assert!(matches!(error, PolicyError::ToolsNotList));
}

fn format_native_malformed() {
    assert_mapped(
        FORMAT_REQUEST,
        TOOLS_X,
        "gpt-4o",
        "2024-08-01",
        MALFORMED_NATIVE,
    );
}

const ORDER_CHOICE: [(&str, &str); 2] = [
    (
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"}}},"tool_choice":"auto"}"#,
        r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":"auto","json_mode":true}"#,
    ),
    (
        r#"{"tool_choice":"auto","response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"}}}}"#,
        r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#,
    ),
];

const ORDER_TOOLS: [(&str, &str); 2] = [
    (
        r#"{"tools":[{"type":"function","function":{"name":"a"}}],"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"}}}}"#,
        r#"{"tools":[{"type":"function","function":{"name":"a"}},{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#,
    ),
    (
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"}}},"tools":[{"type":"function","function":{"name":"a"}}]}"#,
        r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}},{"type":"function","function":{"name":"a"}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#,
    ),
];

fn format_order() {
    order_choice();
    order_tools();
}

fn order_choice() {
    for (non_default, expected) in ORDER_CHOICE {
        assert_mapped(non_default, EMPTY, "gpt-3.5-turbo", "", expected);
    }
}

fn order_tools() {
    for (non_default, expected) in ORDER_TOOLS {
        assert_mapped(non_default, EMPTY, "gpt-3.5-turbo", "", expected);
    }
}
