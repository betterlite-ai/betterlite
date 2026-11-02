use super::*;

// Every message being role system is not the same as there being no system message: the policy still runs and converts it.
#[test]
fn system_placement_behavior_tests_behavior() {
    let messages = vec![text_message("system", "note")];
    assert_eq!(
        place_mid_conversation_system(&messages, false),
        vec![parts_message(
            "user",
            vec![
                text_part(text_parts::CONVERTED_SYSTEM_NOTE),
                text_part("note")
            ]
        )]
    );
}
