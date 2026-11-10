use betterlite_error::envelope::WireField;

use crate::azure_chat_base_policy::mapping::{
    format_supported_for_version, requires_max_completion_tokens, supports_response_format,
};
use crate::azure_chat_base_policy::{PolicyError, SUPPORTED_OPENAI_PARAMS};

use super::support::{
    REQUIRED_UPGRADE, TOOL_UPGRADE, blocked, global_policy, mapped, object, policy,
    required_tool_choice_error, tool_choice_error,
};

pub(super) fn api_version_response_format() {
    response_format_models();
    response_format_versions();
    response_format_native();
    response_format_tool_call();
    response_format_silent_and_invalid();
}

fn response_format_models() {
    let cases = [
        ("gpt-4o", true),
        ("gpt-4.1", true),
        ("gpt-4-1", true),
        ("azure/gpt-4-1", true),
        ("gpt-3.5-turbo", false),
        ("gpt-3-5-turbo", false),
        ("gpt-35-turbo", false),
        ("azure/gpt-3-5", false),
    ];
    for (model, expected) in cases {
        assert_eq!(supports_response_format(model), expected, "{model}");
    }
}

fn response_format_versions() {
    let cases = [
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
    for (year, month, expected) in cases {
        assert_eq!(
            format_supported_for_version(year, month).unwrap(),
            expected,
            "{year}-{month}"
        );
    }
    assert!(matches!(
        format_supported_for_version("2024", "x"),
        Err(PolicyError::ApiVersionNumber { field: "month", .. })
    ));
    assert!(matches!(
        format_supported_for_version("x", "8"),
        Err(PolicyError::ApiVersionNumber { field: "year", .. })
    ));
}

const RESPONSE_FORMAT: &str = r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"}}}}"#;

const EMULATED_RESPONSE_FORMAT: &str = r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#;

fn response_format_native() {
    assert_eq!(
        mapped(
            &object(RESPONSE_FORMAT),
            "{}",
            &policy("gpt-4o", "2024-08-01", false)
        ),
        object(RESPONSE_FORMAT)
    );
    assert_eq!(
        mapped(
            &object(RESPONSE_FORMAT),
            "{}",
            &policy("gpt-4o", "2024-08", false)
        ),
        object(RESPONSE_FORMAT)
    );
    assert_eq!(
        mapped(&object(RESPONSE_FORMAT), "{}", &policy("gpt-4o", "", false)),
        object(RESPONSE_FORMAT)
    );
}

fn response_format_tool_call() {
    assert_eq!(
        mapped(
            &object(RESPONSE_FORMAT),
            "{}",
            &policy("gpt-3.5-turbo", "2024-08-01", false)
        ),
        object(EMULATED_RESPONSE_FORMAT)
    );
    assert_eq!(
        mapped(
            &object(RESPONSE_FORMAT),
            "{}",
            &policy("gpt-4o", "2024-07-01", false)
        ),
        object(EMULATED_RESPONSE_FORMAT)
    );
    assert_eq!(
        mapped(
            &object(RESPONSE_FORMAT),
            "{}",
            &policy("gpt-35-turbo", "2025-01-01", false)
        ),
        object(EMULATED_RESPONSE_FORMAT)
    );
}

fn response_format_silent_and_invalid() {
    let empty_schema =
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{}}}}"#;
    assert_eq!(
        mapped(
            &object(empty_schema),
            "{}",
            &policy("gpt-3.5-turbo", "2024-08-01", false)
        ),
        object("{}")
    );
    assert_eq!(
        mapped(
            &object(r#"{"response_format":"json_object"}"#),
            "{}",
            &policy("gpt-3.5-turbo", "2024-08-01", false)
        ),
        object(r#"{"response_format":"json_object"}"#)
    );
    assert!(matches!(
        blocked(
            &object(RESPONSE_FORMAT),
            "{}",
            &policy("gpt-4o", "abcd-08-01", false)
        ),
        PolicyError::ApiVersionNumber { field: "year", .. }
    ));
    assert!(matches!(
        blocked(
            &object(RESPONSE_FORMAT),
            "{}",
            &policy("gpt-4o", "2024-ab-01", false)
        ),
        PolicyError::ApiVersionNumber { field: "month", .. }
    ));
}

pub(super) fn max_completion_tokens() {
    max_completion_predicates();
    max_completion_mapping();
    max_completion_existing();
}

fn max_completion_predicates() {
    let cases = [
        ("gpt-5", true),
        ("azure/gpt-5-chat", true),
        ("gpt-6-mini", true),
        ("gpt5_series/model", true),
        ("GPT-5", false),
        ("gpt-4o", false),
        ("o1", false),
        ("gpt-4-5", false),
    ];
    for (model, expected) in cases {
        assert_eq!(requires_max_completion_tokens(model), expected, "{model}");
    }
}

fn max_completion_mapping() {
    max_completion_renames();
    max_completion_order();
}

fn max_completion_renames() {
    assert_eq!(
        mapped(
            &object(r#"{"max_tokens":100}"#),
            "{}",
            &policy("gpt-5", "", false)
        ),
        object(r#"{"max_completion_tokens":100}"#)
    );
    assert_eq!(
        mapped(
            &object(r#"{"max_tokens":100}"#),
            "{}",
            &policy("azure/gpt-5-chat", "", false)
        ),
        object(r#"{"max_completion_tokens":100}"#)
    );
    assert_eq!(
        mapped(
            &object(r#"{"max_tokens":100}"#),
            "{}",
            &policy("gpt-4o", "", false)
        ),
        object(r#"{"max_tokens":100}"#)
    );
    assert_eq!(
        mapped(
            &object(r#"{"max_tokens":100,"temperature":0.2}"#),
            "{}",
            &policy("gpt-5", "", false)
        ),
        object(r#"{"max_completion_tokens":100,"temperature":0.2}"#)
    );
}

fn max_completion_order() {
    assert_eq!(
        mapped(
            &object(r#"{"max_completion_tokens":7}"#),
            "{}",
            &policy("gpt-5", "", false)
        ),
        object(r#"{"max_completion_tokens":7}"#)
    );
    assert_eq!(
        mapped(
            &object(r#"{"max_tokens":100}"#),
            r#"{"max_completion_tokens":7}"#,
            &policy("gpt-5", "", false)
        ),
        object(r#"{"max_completion_tokens":7}"#)
    );
    assert_eq!(
        mapped(
            &object(r#"{"max_tokens":100,"max_completion_tokens":7}"#),
            "{}",
            &policy("gpt-5", "", false)
        ),
        object(r#"{"max_completion_tokens":7}"#)
    );
    assert_eq!(
        mapped(
            &object(r#"{"max_completion_tokens":7,"max_tokens":100}"#),
            "{}",
            &policy("gpt-5", "", false)
        ),
        object(r#"{"max_completion_tokens":7}"#)
    );
}

fn max_completion_existing() {
    assert_eq!(
        mapped(
            &object(r#"{"max_tokens":100,"temperature":0.1}"#),
            r#"{"max_tokens":5}"#,
            &policy("gpt-5", "", false)
        ),
        object(r#"{"max_tokens":5,"max_completion_tokens":100,"temperature":0.1}"#)
    );
    assert_eq!(
        mapped(
            &object(r#"{"max_tokens":100}"#),
            "{}",
            &policy("GPT-5", "", false)
        ),
        object(r#"{"max_tokens":100}"#)
    );
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
    let every = r#"{"temperature":0.5,"n":2,"stream":true,"stream_options":{"include_usage":true},"stop":"","max_completion_tokens":4,"presence_penalty":0,"frequency_penalty":0,"logit_bias":{},"user":"u","function_call":"auto","functions":[],"top_p":0.9,"logprobs":true,"top_logprobs":2,"response_format":"json_object","seed":1,"extra_headers":{"x":"y"},"parallel_tool_calls":false,"prediction":{},"modalities":["text"],"audio":{},"web_search_options":{},"prompt_cache_key":"p","store":true,"unknown_param":1}"#;
    let kept = r#"{"temperature":0.5,"n":2,"stream":true,"stream_options":{"include_usage":true},"stop":"","max_completion_tokens":4,"presence_penalty":0,"frequency_penalty":0,"logit_bias":{},"user":"u","function_call":"auto","functions":[],"top_p":0.9,"logprobs":true,"top_logprobs":2,"response_format":"json_object","seed":1,"extra_headers":{"x":"y"},"parallel_tool_calls":false,"prediction":{},"modalities":["text"],"audio":{},"web_search_options":{},"prompt_cache_key":"p","store":true}"#;
    assert_eq!(
        mapped(&object(every), "{}", &policy("gpt-4o", "", false)),
        object(kept)
    );
    assert_eq!(
        mapped(
            &object(r#"{"unknown_param":1}"#),
            "{}",
            &policy("gpt-4o", "", false)
        ),
        object("{}")
    );
    assert_eq!(
        mapped(
            &object(r#"{"temperature":null}"#),
            "{}",
            &policy("gpt-4o", "", false)
        ),
        object(r#"{"temperature":null}"#)
    );
}

pub(super) fn azure_tool_choice_date_gate() {
    date_gate_rejection();
    date_gate_allowed();
    date_gate_dropped();
}

fn date_gate_rejection() {
    let error = tool_choice_error("2022-01-01");
    assert_eq!(error.to_string(), TOOL_UPGRADE);
    assert_eq!(error.status_code(), Some(400));
    let metadata = error.classify().unwrap().metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::MISSING);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.code, "400");
    for api_version in ["2023-11-30", "2023-12-00", "2022-12-31", "0000-01-01"] {
        assert!(
            matches!(
                tool_choice_error(api_version),
                PolicyError::ToolChoiceUnsupported { .. }
            ),
            "{api_version}"
        );
    }
    assert_eq!(
        blocked(
            &object(r#"{"tool_choice":null}"#),
            "{}",
            &policy("gpt-4o", "2023-11-30", false)
        )
        .status_code(),
        Some(400)
    );
    assert!(PolicyError::ToolsNotList.classify().is_none());
}

fn date_gate_allowed() {
    for api_version in [
        "2023-12-01",
        "2023-12-02",
        "2023-12-1",
        "2023-2-01",
        "2023-13-01",
        "2024-01-01",
        "2023-11",
        "",
        "abc",
        "not-a-date",
    ] {
        assert_eq!(
            mapped(
                &object(r#"{"tool_choice":"auto"}"#),
                "{}",
                &policy("gpt-4o", api_version, false)
            ),
            object(r#"{"tool_choice":"auto"}"#),
            "{api_version}"
        );
    }
}

fn date_gate_dropped() {
    assert_eq!(
        mapped(
            &object(r#"{"tool_choice":"auto"}"#),
            "{}",
            &policy("gpt-4o", "2023-11-30", true)
        ),
        object("{}")
    );
    assert_eq!(
        mapped(
            &object(r#"{"tool_choice":"auto"}"#),
            "{}",
            &global_policy("gpt-4o", "2023-11-30")
        ),
        object("{}")
    );
    assert_eq!(
        mapped(
            &object(r#"{"tool_choice":"auto"}"#),
            "{}",
            &policy("gpt-4o", "2023-12-01", true)
        ),
        object(r#"{"tool_choice":"auto"}"#)
    );
}

pub(super) fn azure_required_tool_choice() {
    required_rejection();
    required_allowed();
    required_dropped();
}

fn required_rejection() {
    let error = required_tool_choice_error("2024-05-01");
    assert_eq!(error.to_string(), REQUIRED_UPGRADE);
    assert_eq!(error.status_code(), Some(400));
    let metadata = error.classify().unwrap().metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::MISSING);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.code, "400");
    for api_version in ["2024-05-31", "2024-04-01", "2024-02-01", "2024-05-01"] {
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
    for api_version in ["2024-5-01", "2024-06-01", "2023-12-01", "2023-12-1"] {
        assert_eq!(
            mapped(
                &object(r#"{"tool_choice":"required"}"#),
                "{}",
                &policy("gpt-4o", api_version, false)
            ),
            object(r#"{"tool_choice":"required"}"#),
            "{api_version}"
        );
    }
    for value in [r#""""#, "false", r#""none""#] {
        let non_default = object(&format!(r#"{{"tool_choice":{value}}}"#));
        assert_eq!(
            mapped(&non_default, "{}", &policy("gpt-4o", "2024-05-01", false)),
            object(&format!(r#"{{"tool_choice":{value}}}"#)),
            "{value}"
        );
    }
    assert_eq!(
        mapped(
            &object(r#"{"tool_choice":"auto"}"#),
            "{}",
            &policy("gpt-4o", "2024-05-01", false)
        ),
        object(r#"{"tool_choice":"auto"}"#)
    );
}

fn required_dropped() {
    assert_eq!(
        mapped(
            &object(r#"{"tool_choice":"required"}"#),
            "{}",
            &policy("gpt-4o", "2024-05-01", true)
        ),
        object("{}")
    );
    assert_eq!(
        mapped(
            &object(r#"{"tool_choice":"required"}"#),
            "{}",
            &global_policy("gpt-4o", "2024-05-01")
        ),
        object("{}")
    );
}

pub(super) fn azure_parameter_shapes() {
    tools_shapes();
    response_format_shapes();
    mapping_order();
}

fn tools_shapes() {
    let incoming = object(r#"{"tools":[{"type":"function","function":{"name":"b"}}]}"#);
    assert_eq!(
        mapped(
            &incoming,
            r#"{"tools":[{"type":"function","function":{"name":"a"}}]}"#,
            &policy("gpt-4o", "", false)
        ),
        object(
            r#"{"tools":[{"type":"function","function":{"name":"a"}},{"type":"function","function":{"name":"b"}}]}"#
        )
    );
    assert!(matches!(
        blocked(&incoming, r#"{"tools":"x"}"#, &policy("gpt-4o", "", false)),
        PolicyError::ToolsNotList
    ));
    assert!(matches!(
        blocked(&incoming, r#"{"tools":null}"#, &policy("gpt-4o", "", false)),
        PolicyError::ToolsNotList
    ));
    assert_eq!(
        mapped(
            &object(r#"{"tools":{"a":1}}"#),
            "{}",
            &policy("gpt-4o", "", false)
        ),
        object(r#"{"tools":{"a":1}}"#)
    );
    assert_eq!(
        mapped(
            &object(r#"{"tools":{"a":1}}"#),
            r#"{"tools":[]}"#,
            &policy("gpt-4o", "", false)
        ),
        object(r#"{"tools":{"a":1}}"#)
    );
}

const JSON_SCHEMA: &str =
    r#"{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"}}}"#;

fn response_format_shapes() {
    response_format_tools();
    response_format_errors();
    response_format_passthrough();
}

fn response_format_tools() {
    let format = object(&format!(r#"{{"response_format":{JSON_SCHEMA}}}"#));
    assert_eq!(
        mapped(
            &format,
            r#"{"tools":[{"type":"function","function":{"name":"a"}}]}"#,
            &policy("gpt-3.5-turbo", "2024-08-01", false)
        ),
        object(
            r#"{"tools":[{"type":"function","function":{"name":"a"}},{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#
        )
    );
    let precedence = r#"{"response_format":{"response_schema":{"type":"object","properties":{"a":{"type":"string"}}},"json_schema":{"name":"n","schema":{"type":"string"}}}}"#;
    assert_eq!(
        mapped(
            &object(precedence),
            "{}",
            &policy("gpt-3.5-turbo", "2024-08-01", false)
        ),
        object(
            r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object","properties":{"a":{"type":"string"}}}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#
        )
    );
    let empty_schema =
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{}}}}"#;
    assert_eq!(
        mapped(
            &object(empty_schema),
            "{}",
            &policy("gpt-4o", "2024-08-01", false)
        ),
        object(empty_schema)
    );
}

fn response_format_errors() {
    let format = object(&format!(r#"{{"response_format":{JSON_SCHEMA}}}"#));
    assert!(matches!(
        blocked(
            &object(r#"{"response_format":{"json_schema":{"name":"n"}}}"#),
            "{}",
            &policy("gpt-3.5-turbo", "2024-08-01", false)
        ),
        PolicyError::ResponseFormat { .. }
    ));
    assert!(matches!(
        blocked(
            &object(r#"{"response_format":{"json_schema":"x"}}"#),
            "{}",
            &policy("gpt-3.5-turbo", "2024-08-01", false)
        ),
        PolicyError::ResponseFormat { .. }
    ));
    assert!(matches!(
        blocked(
            &format,
            r#"{"tools":"x"}"#,
            &policy("gpt-3.5-turbo", "2024-08-01", false)
        ),
        PolicyError::ToolsNotList
    ));
    assert_eq!(
        mapped(
            &format,
            r#"{"tools":"x"}"#,
            &policy("gpt-4o", "2024-08-01", false)
        ),
        object(
            r#"{"tools":"x","response_format":{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"}}}}"#
        )
    );
}

fn response_format_passthrough() {
    assert_eq!(
        mapped(
            &object(r#"{"response_format":"json_object"}"#),
            "{}",
            &policy("gpt-3.5-turbo", "2024-08-01", false)
        ),
        object(r#"{"response_format":"json_object"}"#)
    );
}

fn mapping_order() {
    choice_and_format_order();
    tools_and_format_order();
}

fn choice_and_format_order() {
    let format_then_choice = object(&format!(
        r#"{{"response_format":{JSON_SCHEMA},"tool_choice":"auto"}}"#
    ));
    assert_eq!(
        mapped(
            &format_then_choice,
            "{}",
            &policy("gpt-3.5-turbo", "", false)
        ),
        object(
            r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":"auto","json_mode":true}"#
        )
    );
    let choice_then_format = object(&format!(
        r#"{{"tool_choice":"auto","response_format":{JSON_SCHEMA}}}"#
    ));
    assert_eq!(
        mapped(
            &choice_then_format,
            "{}",
            &policy("gpt-3.5-turbo", "", false)
        ),
        object(
            r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#
        )
    );
}

fn tools_and_format_order() {
    let tools_then_format = object(&format!(
        r#"{{"tools":[{{"type":"function","function":{{"name":"a"}}}}],"response_format":{JSON_SCHEMA}}}"#
    ));
    assert_eq!(
        mapped(
            &tools_then_format,
            "{}",
            &policy("gpt-3.5-turbo", "", false)
        ),
        object(
            r#"{"tools":[{"type":"function","function":{"name":"a"}},{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#
        )
    );
    let format_then_tools = object(&format!(
        r#"{{"response_format":{JSON_SCHEMA},"tools":[{{"type":"function","function":{{"name":"a"}}}}]}}"#
    ));
    assert_eq!(
        mapped(
            &format_then_tools,
            "{}",
            &policy("gpt-3.5-turbo", "", false)
        ),
        object(
            r#"{"tools":[{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}},{"type":"function","function":{"name":"a"}}],"tool_choice":{"type":"function","function":{"name":"json_tool_call"}},"json_mode":true}"#
        )
    );
}
