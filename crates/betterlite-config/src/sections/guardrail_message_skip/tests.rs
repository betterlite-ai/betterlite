#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::guardrail_message_skip::Settings;

#[rstest]
#[case::absent("", false, false)]
#[case::null(
    "litellm_settings: {skip_system_message_in_guardrail: null, skip_tool_message_in_guardrail: null}",
    false,
    false
)]
#[case::system_only(
    "litellm_settings: {skip_system_message_in_guardrail: true}",
    true,
    false
)]
#[case::tool_only(
    "litellm_settings: {skip_tool_message_in_guardrail: true}",
    false,
    true
)]
#[case::both(
    "litellm_settings: {skip_system_message_in_guardrail: true, skip_tool_message_in_guardrail: true}",
    true,
    true
)]
#[case::explicit_false(
    "litellm_settings: {skip_system_message_in_guardrail: false, skip_tool_message_in_guardrail: false}",
    false,
    false
)]
fn skip_flags_are_read_from_litellm_settings(
    #[case] text: &str,
    #[case] system: bool,
    #[case] tool: bool,
) {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let settings = loader.load(text).unwrap().sections.guardrail_message_skip;
    assert_eq!(
        settings,
        Settings {
            skip_system_message: system,
            skip_tool_message: tool,
        }
    );
}

#[test]
fn skip_flag_that_is_not_a_boolean_fails_the_load() {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let text = "litellm_settings: {skip_tool_message_in_guardrail: [true]}";
    loader.load(text).unwrap_err();
}
