use rstest::rstest;

use crate::arguments::json::{Json, Object};
use crate::cache_accounting::points::CacheControlInjectionPoint;
use crate::cache_defaults::CachePolicy;
use crate::cache_defaults::selection::{DefaultRequest, get_default_injection_points};
use crate::cache_placement::system::SystemPrompt;
use crate::message_content::Message;

use super::support::{
    BARE_MODEL, DEFAULT_POINTS, Facts, MARKED_SYSTEM, MODEL, json, messages, object, system, wire,
};

struct Setup {
    model: String,
    custom_llm_provider: Option<String>,
    messages: Vec<Message>,
    system: Option<SystemPrompt>,
    tools: Option<Vec<Json>>,
    enable: Option<bool>,
    cache_control: Option<Json>,
    kwargs: Object,
    on_route: bool,
    global: bool,
    ttl: Option<String>,
}

impl Setup {
    fn claude() -> Self {
        Self {
            model: MODEL.to_owned(),
            custom_llm_provider: None,
            messages: messages(r#"[{"role": "user", "content": "hi"}]"#),
            system: None,
            tools: None,
            enable: None,
            cache_control: None,
            kwargs: Object::new(),
            on_route: false,
            global: true,
            ttl: None,
        }
    }
}

fn defaults(setup: &Setup, facts: &Facts) -> Vec<CacheControlInjectionPoint> {
    get_default_injection_points(
        &DefaultRequest {
            model: &setup.model,
            custom_llm_provider: setup.custom_llm_provider.as_deref(),
            messages: &setup.messages,
            system: setup.system.as_ref(),
            tools: setup.tools.as_deref(),
            enable_prompt_caching: setup.enable,
            cache_control: setup.cache_control.as_ref(),
            request_kwargs: &setup.kwargs,
            on_messages_route: setup.on_route,
            policy: CachePolicy {
                global_enabled: setup.global,
                ttl: setup.ttl.as_deref(),
            },
        },
        facts,
    )
}

#[test]
fn defaults_are_not_selected_when_both_flags_are_off() {
    let setup = Setup {
        global: false,
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[test]
fn an_explicit_false_per_request_flag_does_not_enable_defaults() {
    let setup = Setup {
        global: false,
        enable: Some(false),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[test]
fn the_per_request_flag_enables_defaults_with_the_global_flag_off() {
    let setup = Setup {
        global: false,
        enable: Some(true),
        ..Setup::claude()
    };
    assert_eq!(
        wire(&defaults(&setup, &Facts::claude())),
        json(DEFAULT_POINTS)
    );
}

#[test]
fn defaults_stand_down_for_a_model_without_prompt_caching() {
    let setup = Setup::claude();
    assert!(defaults(&setup, &Facts::without_support()).is_empty());
}

#[rstest]
#[case::not_claude(
    "anthropic/llama-3-70b",
    Facts::routed("anthropic/llama-3-70b", "anthropic")
)]
#[case::unroutable("somewhere/unknown-model", Facts::claude())]
fn defaults_stand_down_for_a_model_that_is_not_claude(#[case] model: &str, #[case] facts: Facts) {
    let setup = Setup {
        model: model.to_owned(),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &facts).is_empty());
}

#[test]
fn an_explicit_custom_provider_resolves_the_capability() {
    let setup = Setup {
        model: BARE_MODEL.to_owned(),
        custom_llm_provider: Some("anthropic".to_owned()),
        ..Setup::claude()
    };
    assert_eq!(
        wire(&defaults(&setup, &Facts::claude())),
        json(DEFAULT_POINTS)
    );
}

#[test]
fn an_explicit_non_anthropic_provider_stands_down() {
    let setup = Setup {
        model: BARE_MODEL.to_owned(),
        custom_llm_provider: Some("openai".to_owned()),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[rstest]
#[case::message_mark(
    r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#
)]
#[case::content_block_mark(r#"[{"role": "user", "content": [{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}]}]"#)]
#[case::prompt_breakpoint_mark(r#"[{"role": "user", "content": [{"type": "text", "text": "hi", "prompt_cache_breakpoint": {"mode": "explicit"}}]}]"#)]
fn client_marks_on_messages_suppress_defaults(#[case] conversation: &str) {
    let setup = Setup {
        messages: messages(conversation),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[test]
fn client_marks_on_system_blocks_suppress_defaults() {
    let setup = Setup {
        system: system(MARKED_SYSTEM),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[test]
fn a_client_mark_on_a_tool_suppresses_defaults() {
    let setup = Setup {
        tools: Some(vec![json(
            r#"{"type": "custom", "name": "t", "cache_control": {"type": "ephemeral"}}"#,
        )]),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[test]
fn a_top_level_cache_control_suppresses_defaults() {
    let setup = Setup {
        cache_control: Some(json(r#"{"type": "ephemeral"}"#)),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[test]
fn a_null_top_level_cache_control_is_not_a_mark() {
    let setup = Setup {
        cache_control: Some(Json::Null),
        ..Setup::claude()
    };
    assert_eq!(
        wire(&defaults(&setup, &Facts::claude())),
        json(DEFAULT_POINTS)
    );
}

#[test]
fn a_mark_in_the_extra_body_envelope_suppresses_defaults() {
    let setup = Setup {
        kwargs: object(r#"{"extra_body": {"cache_control": {"type": "ephemeral"}}}"#),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[test]
fn the_messages_route_takes_the_larger_envelope_census() {
    let setup = Setup {
        tools: Some(vec![json(r#"{"type": "custom", "name": "t"}"#)]),
        kwargs: object(
            r#"{"extra_body": {"tools": [{"type": "custom", "name": "t", "cache_control": {"type": "ephemeral"}}]}}"#,
        ),
        on_route: true,
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

const SUBAGENT_SYSTEM: &str = "x-anthropic-billing-header: cc_version=2.0.1; cc_is_subagent=true";
const CLAUDE_CODE_HEADERS: &str =
    r#"{"proxy_server_request": {"headers": {"User-Agent": "claude-cli/2.0.1"}}}"#;

#[test]
fn a_claude_code_one_shot_subagent_request_is_suppressed() {
    let setup = Setup {
        system: Some(SystemPrompt::Text(SUBAGENT_SYSTEM.to_owned())),
        kwargs: object(CLAUDE_CODE_HEADERS),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[test]
fn the_billing_header_may_arrive_in_a_system_block() {
    let setup = Setup {
        system: system(
            r#"[{"type": "text", "text": "x-anthropic-billing-header: cc_is_subagent=true"}]"#,
        ),
        kwargs: object(CLAUDE_CODE_HEADERS),
        ..Setup::claude()
    };
    assert!(defaults(&setup, &Facts::claude()).is_empty());
}

#[rstest]
#[case::a_second_message(
    r#"[{"role": "user", "content": "hi"}, {"role": "user", "content": "again"}]"#
)]
#[case::a_non_user_role(r#"[{"role": "assistant", "content": "hi"}]"#)]
fn a_one_shot_request_needs_a_single_user_message(#[case] conversation: &str) {
    let setup = Setup {
        messages: messages(conversation),
        system: Some(SystemPrompt::Text(SUBAGENT_SYSTEM.to_owned())),
        kwargs: object(CLAUDE_CODE_HEADERS),
        ..Setup::claude()
    };
    assert_eq!(
        wire(&defaults(&setup, &Facts::claude())),
        json(DEFAULT_POINTS)
    );
}

#[test]
fn tools_disarm_the_one_shot_suppression() {
    let setup = Setup {
        system: Some(SystemPrompt::Text(SUBAGENT_SYSTEM.to_owned())),
        kwargs: object(CLAUDE_CODE_HEADERS),
        tools: Some(vec![json(r#"{"type": "custom", "name": "t"}"#)]),
        ..Setup::claude()
    };
    assert_eq!(
        wire(&defaults(&setup, &Facts::claude())),
        json(DEFAULT_POINTS)
    );
}

#[rstest]
#[case::another_client("curl/8.0")]
#[case::no_separator("claude")]
fn a_non_claude_code_user_agent_is_not_one_shot(#[case] user_agent: &str) {
    let kwargs = object(&format!(
        r#"{{"proxy_server_request": {{"headers": {{"User-Agent": "{user_agent}"}}}}}}"#
    ));
    let setup = Setup {
        system: Some(SystemPrompt::Text(SUBAGENT_SYSTEM.to_owned())),
        kwargs,
        ..Setup::claude()
    };
    assert_eq!(
        wire(&defaults(&setup, &Facts::claude())),
        json(DEFAULT_POINTS)
    );
}

#[rstest]
#[case::field_absent("x-anthropic-billing-header: cc_version=2.0.1")]
#[case::field_false("x-anthropic-billing-header: cc_is_subagent=false")]
#[case::two_values("x-anthropic-billing-header: cc_is_subagent=true; cc_is_subagent=true")]
#[case::missing_equals("x-anthropic-billing-header: cc_is_subagent")]
#[case::blank_value("x-anthropic-billing-header: cc_is_subagent=")]
#[case::newline_in_value("x-anthropic-billing-header: cc_is_subagent=\ntrue")]
#[case::carriage_return_in_value("x-anthropic-billing-header: cc_is_subagent=\rtrue")]
#[case::empty_key_then_marker("x-anthropic-billing-header: =x; cc_is_subagent=true")]
#[case::empty_value_then_marker("x-anthropic-billing-header: a=; cc_is_subagent=true")]
fn a_malformed_billing_header_is_not_one_shot(#[case] text: &str) {
    assert!(text.starts_with("x-anthropic-billing-header:"));
    let request = Setup {
        system: Some(SystemPrompt::Text(text.to_owned())),
        kwargs: object(CLAUDE_CODE_HEADERS),
        ..Setup::claude()
    };
    assert_eq!(
        wire(&defaults(&request, &Facts::claude())),
        json(DEFAULT_POINTS)
    );
}

#[test]
fn a_system_block_that_is_not_text_disarms_the_one_shot_suppression() {
    let setup = Setup {
        system: system(
            r#"[{"type": "text", "text": "x-anthropic-billing-header: cc_is_subagent=true"}, {"type": "image", "source": {}}]"#,
        ),
        kwargs: object(CLAUDE_CODE_HEADERS),
        ..Setup::claude()
    };
    assert_eq!(
        wire(&defaults(&setup, &Facts::claude())),
        json(DEFAULT_POINTS)
    );
}

#[test]
fn a_multiline_billing_header_is_not_one_shot() {
    let setup = Setup {
        system: Some(SystemPrompt::Text(format!("{SUBAGENT_SYSTEM}\nextra"))),
        kwargs: object(CLAUDE_CODE_HEADERS),
        ..Setup::claude()
    };
    assert_eq!(
        wire(&defaults(&setup, &Facts::claude())),
        json(DEFAULT_POINTS)
    );
}

const EPHEMERAL: &str = r#"{"type": "ephemeral"}"#;

#[rstest]
#[case::unset(None, EPHEMERAL)]
#[case::five_minutes(Some("5m"), r#"{"type": "ephemeral", "ttl": "5m"}"#)]
#[case::one_hour(Some("1h"), r#"{"type": "ephemeral", "ttl": "1h"}"#)]
#[case::other_value(Some("2h"), EPHEMERAL)]
#[case::empty_value(Some(""), EPHEMERAL)]
fn the_ttl_policy_reaches_both_default_points(#[case] ttl: Option<&str>, #[case] control: &str) {
    let setup = Setup {
        ttl: ttl.map(str::to_owned),
        ..Setup::claude()
    };
    let expected = format!(
        r#"[
            {{"location": "message", "role": "system", "index": null, "control": {control}}},
            {{"location": "message", "role": null, "index": -1, "control": {control}}}
        ]"#
    );
    assert_eq!(wire(&defaults(&setup, &Facts::claude())), json(&expected));
}
