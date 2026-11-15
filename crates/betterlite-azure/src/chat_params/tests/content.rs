use super::support::{TOOLS, composed, o_series_body, object};

const MESSAGES: &str = r#"[{"role":"user","content":"hi"}]"#;
const SYSTEM_FIRST: &str =
    r#"[{"role":"system","content":"be brief"},{"role":"user","content":"hi"}]"#;
const SYSTEM_AFTER_USER: &str =
    r#"[{"role":"user","content":"hi"},{"role":"system","content":"be brief"}]"#;
const SANITIZED_TOOLS: &str = r#"[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{"a":{"type":"string"}}}}},{"type":"function","function":{"name":"g","parameters":{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"number"}}}}}]"#;

// (model, optional params, messages, openai_system_messages_first, expected body)
const O_SERIES_BODIES: [(&str, &str, &str, bool, &str); 6] = [
    (
        "o_series/deploy/o_series/my-model",
        r#"{"temperature":0.1}"#,
        MESSAGES,
        false,
        r#"{"model":"deploy/my-model","messages":[{"role":"user","content":"hi"}],"temperature":0.1}"#,
    ),
    (
        "o_series/o1-mini",
        "{}",
        SYSTEM_FIRST,
        false,
        r#"{"model":"o1-mini","messages":[{"role":"user","content":"be brief"},{"role":"user","content":"hi"}]}"#,
    ),
    (
        "o_series/o1-mini",
        "{}",
        SYSTEM_AFTER_USER,
        true,
        r#"{"model":"o1-mini","messages":[{"role":"user","content":"be brief"},{"role":"user","content":"hi"}]}"#,
    ),
    (
        "o_series/o1-mini",
        r#"{"max_retries":3,"tool_choice":"auto"}"#,
        SYSTEM_AFTER_USER,
        false,
        r#"{"model":"o1-mini","messages":[{"role":"user","content":"hi"},{"role":"user","content":"be brief"}]}"#,
    ),
    (
        "o_series/o1-mini",
        "{}",
        r#"[{"role":"user","content":"hi","cache_control":{"type":"ephemeral"}}]"#,
        false,
        r#"{"model":"o1-mini","messages":[{"role":"user","content":"hi"}]}"#,
    ),
    (
        "o_series/o1-mini",
        "{}",
        r#"[{"role":"assistant","function_call":{"name":"f"}}]"#,
        false,
        r#"{"model":"o1-mini","messages":[{"role":"assistant","function_call":{"name":"f"}}]}"#,
    ),
];

pub(super) fn gpt5_base_policy_composition() {
    assert_eq!(
        composed(
            "gpt-5.1",
            &[r#"{"tools":"#, TOOLS, "}"].concat(),
            SYSTEM_AFTER_USER,
            true
        ),
        object(
            &[
                r#"{"model":"gpt-5.1","messages":[{"role":"system","content":"be brief"},{"role":"user","content":"hi"}],"tools":"#,
                SANITIZED_TOOLS,
                "}",
            ]
            .concat()
        )
    );
    assert_eq!(
        composed("gpt-5.1", r#"{"tool_choice":"auto"}"#, MESSAGES, true),
        object(r#"{"model":"gpt-5.1","messages":[{"role":"user","content":"hi"}]}"#)
    );
    tools_with_choice_and_effort();
}

fn tools_with_choice_and_effort() {
    let requested = [
        r#"{"tools":"#,
        TOOLS,
        r#","tool_choice":"auto","reasoning_effort":"high"}"#,
    ]
    .concat();
    let expected = [
        r#"{"model":"gpt-5.1","messages":[{"role":"user","content":"hi"}],"reasoning_effort":"high","tool_choice":"auto","tools":"#,
        SANITIZED_TOOLS,
        "}",
    ]
    .concat();
    assert_eq!(
        composed("gpt-5.1", &requested, MESSAGES, true),
        object(&expected)
    );
}

pub(super) fn azure_reasoning_route_strip() {
    gpt5_route_strip();
    o_series_tools_route_strip();
    for (model, optional, messages, system_first, expected) in O_SERIES_BODIES {
        assert_eq!(
            o_series_body(model, optional, messages, system_first),
            object(expected)
        );
    }
}

fn gpt5_route_strip() {
    let requested = [r#"{"tools":"#, TOOLS, r#","reasoning_effort":"high"}"#].concat();
    let expected = [
        r#"{"model":"gpt-5.1","messages":[{"role":"user","content":"hi"}],"reasoning_effort":"high","tools":"#,
        SANITIZED_TOOLS,
        "}",
    ]
    .concat();
    assert_eq!(
        composed("gpt5_series/gpt-5.1", &requested, MESSAGES, false),
        object(&expected)
    );
    assert_eq!(
        composed("deploy/gpt5_series/gpt-5.1", "{}", MESSAGES, false),
        object(r#"{"model":"deploy/gpt-5.1","messages":[{"role":"user","content":"hi"}]}"#)
    );
    assert_eq!(
        composed("gpt5_series/gpt-5.2", "{}", MESSAGES, false),
        object(r#"{"model":"gpt-5.2","messages":[{"role":"user","content":"hi"}]}"#)
    );
}

fn o_series_tools_route_strip() {
    let requested = [r#"{"tools":"#, TOOLS, r#","temperature":0.1}"#].concat();
    let expected = [
        r#"{"model":"my-model","messages":[{"role":"user","content":"hi"}],"temperature":0.1,"tools":"#,
        SANITIZED_TOOLS,
        "}",
    ]
    .concat();
    assert_eq!(
        o_series_body("o_series/my-model", &requested, MESSAGES, false),
        object(&expected)
    );
}
