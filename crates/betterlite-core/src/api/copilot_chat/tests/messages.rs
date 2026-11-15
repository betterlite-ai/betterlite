use betterlite_config::sections::copilot_chat::Settings;

use crate::api::copilot_chat::messages::{CopilotMessage, transform_messages};

const SENT: &str = r#"[
    {"role":"system","content":"rules","name":"setup","cache_control":{"type":"ephemeral"}},
    {"role":"user","content":[{"type":"text","text":"hi"}]},
    {"role":"developer","content":"dev"},
    {"role":"system","content":null},
    {"content":"no role"}
]"#;

const CONVERTED: &str = r#"[
    {"role":"assistant","content":"rules","name":"setup","cache_control":{"type":"ephemeral"}},
    {"role":"user","content":[{"type":"text","text":"hi"}]},
    {"role":"developer","content":"dev"},
    {"role":"assistant","content":null},
    {"content":"no role"}
]"#;

#[rstest::rstest]
#[case::conversion_on(false, CONVERTED)]
#[case::conversion_disabled(true, SENT)]
fn transform_messages_copies_system_messages_to_assistant_unless_disabled(
    #[case] disabled: bool,
    #[case] expected: &str,
) {
    let settings = Settings {
        disable_system_to_assistant: disabled,
        api_base: None,
    };
    let sent: Vec<CopilotMessage> = serde_json::from_str(SENT).unwrap();
    let converted = transform_messages(sent, &settings);
    let wanted: Vec<CopilotMessage> = serde_json::from_str(expected).unwrap();
    assert_eq!(converted, wanted);
}
