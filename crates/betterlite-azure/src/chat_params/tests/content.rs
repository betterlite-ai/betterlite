use super::support::{TOOLS, composed, o_series_body, object};

const MESSAGES: &str = r#"[{"role":"user","content":"hi"}]"#;
const SYSTEM_AFTER_USER: &str =
    r#"[{"role":"user","content":"hi"},{"role":"system","content":"be brief"}]"#;
const SANITIZED_TOOLS: &str = r#"[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{"a":{"type":"string"}}}}},{"type":"function","function":{"name":"g","parameters":{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"number"}}}}}]"#;

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
    assert_eq!(
        composed(
            "gpt-5.1",
            &[
                r#"{"tools":"#,
                TOOLS,
                r#","tool_choice":"auto","reasoning_effort":"high"}"#,
            ]
            .concat(),
            MESSAGES,
            true
        ),
        object(
            &[
                r#"{"model":"gpt-5.1","messages":[{"role":"user","content":"hi"}],"reasoning_effort":"high","tool_choice":"auto","tools":"#,
                SANITIZED_TOOLS,
                "}",
            ]
            .concat()
        )
    );
}

pub(super) fn azure_reasoning_route_strip() {
    assert_eq!(
        composed(
            "gpt5_series/gpt-5.1",
            &[r#"{"tools":"#, TOOLS, r#","reasoning_effort":"high"}"#].concat(),
            MESSAGES,
            false
        ),
        object(
            &[
                r#"{"model":"gpt-5.1","messages":[{"role":"user","content":"hi"}],"reasoning_effort":"high","tools":"#,
                SANITIZED_TOOLS,
                "}",
            ]
            .concat()
        )
    );
    assert_eq!(
        composed("deploy/gpt5_series/gpt-5.1", "{}", MESSAGES, false),
        object(r#"{"model":"deploy/gpt-5.1","messages":[{"role":"user","content":"hi"}]}"#)
    );
    assert_eq!(
        composed("gpt5_series/gpt-5.2", "{}", MESSAGES, false),
        object(r#"{"model":"gpt-5.2","messages":[{"role":"user","content":"hi"}]}"#)
    );
    assert_eq!(
        o_series_body(
            "o_series/deploy/o_series/my-model",
            r#"{"temperature":0.1}"#,
            MESSAGES
        ),
        object(
            r#"{"model":"deploy/my-model","messages":[{"role":"user","content":"hi"}],"temperature":0.1}"#
        )
    );
    assert_eq!(
        o_series_body(
            "o_series/my-model",
            &[r#"{"tools":"#, TOOLS, r#","temperature":0.1}"#].concat(),
            MESSAGES
        ),
        object(
            &[
                r#"{"model":"my-model","messages":[{"role":"user","content":"hi"}],"temperature":0.1,"tools":"#,
                SANITIZED_TOOLS,
                "}",
            ]
            .concat()
        )
    );
}
