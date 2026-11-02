use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    let messages = vec![text_message("user", "hi"), text_message("assistant", "ok")];
    assert_eq!(place_mid_conversation_system(&messages, true), messages);
    assert_eq!(place_mid_conversation_system(&messages, false), messages);
    assert_eq!(
        place_mid_conversation_system(&[], true),
        Vec::<Message>::new()
    );
    assert_eq!(
        place_mid_conversation_system(&[], false),
        Vec::<Message>::new()
    );
}
