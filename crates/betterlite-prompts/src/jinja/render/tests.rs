#![cfg(test)]

use super::render_chat_template;
use crate::builtin::Message;
use crate::jinja::engine;
use chrono::{TimeZone as _, Utc};
use rstest::rstest;

fn messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).unwrap()
}

#[rstest]
#[case::raise_exception_displays_without_raising(
    "{{ raise_exception('boom') }}",
    r#"[{"role":"user","content":"hi"}]"#,
    Some("Error message - boom")
)]
#[case::system_probe_true_despite_rejection_intent(
    "{%- if messages[0]['role'] == 'system' -%}{{ raise_exception('System role not supported') }}{%- endif -%}{%- for m in messages -%}{{ m['role'] }}:{{ m['content'] }};{%- endfor -%}",
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#,
    Some("Error message - System role not supportedsystem:s;user:u;")
)]
#[case::real_render_error_wraps_the_cause(
    "{%- if add_generation_prompt is defined and add_generation_prompt -%}{{ undefined_helper() }}{%- endif -%}ok",
    r#"[{"role":"user","content":"hi"}]"#,
    None
)]
#[case::alternation_retry_omits_generation_prompt(
    r#"{%- if messages[0]['role'] == 'system' -%}{{ messages["System role not supported"]() }}{%- endif -%}{%- if messages|length == 2 -%}{{ messages["Conversation roles must alternate user/assistant"]() }}{%- endif -%}{%- for m in messages -%}{{ m['role'] }}:{{ m['content'] }};{%- endfor -%}{%- if add_generation_prompt is defined -%}GEN{%- endif -%}"#,
    r#"[{"role":"system","content":"S"},{"role":"user","content":"U"}]"#,
    Some("user:S;assistant:;user:U;")
)]
#[case::python_strip_via_pycompat(
    "{{ messages[0]['content'].strip() }}",
    r#"[{"role":"user","content":"  padded  "}]"#,
    Some("padded")
)]
#[case::unrelated_render_error_is_silently_swallowed(
    "{{ undefined_name() }}",
    r#"[{"role":"user","content":"hi"}]"#,
    Some("")
)]
#[case::alternation_retry_with_no_messages_left_still_errors(
    r#"{{ messages["Conversation roles must alternate user/assistant"]() }}"#,
    "[]",
    None
)]
fn jinja_prompts_behavior_tests_behavior(
    #[case] chat_template: &str,
    #[case] wire: &str,
    #[case] expected: Option<&str>,
) {
    let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let env = engine::build(now);
    let actual = render_chat_template(&env, chat_template, "", "", &messages(wire));
    match expected {
        Some(text) => assert_eq!(actual.unwrap(), text),
        None => {
            actual.unwrap_err();
        }
    }
}
