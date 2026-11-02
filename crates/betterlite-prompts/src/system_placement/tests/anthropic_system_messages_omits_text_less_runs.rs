use super::*;
use crate::system_placement::anthropic_system::{
    AnthropicSystemMessage, anthropic_system_messages,
};

#[test]
fn system_placement_behavior_tests_behavior() {
    let without_text = message("system", None);
    assert_eq!(anthropic_system_messages(&without_text), Vec::new());

    let with_text = text_message("system", "hello");
    assert_eq!(
        anthropic_system_messages(&with_text),
        vec![AnthropicSystemMessage {
            role: "system".to_owned(),
            content: vec![text_part("hello")],
        }]
    );
}
