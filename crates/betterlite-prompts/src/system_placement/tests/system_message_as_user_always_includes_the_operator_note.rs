use super::*;
use crate::system_placement::text_parts::system_message_as_user;

#[test]
fn system_placement_behavior_tests_behavior() {
    let with_text = text_message("system", "reminder");
    assert_eq!(
        system_message_as_user(&with_text),
        parts_message(
            "user",
            vec![
                text_part(text_parts::CONVERTED_SYSTEM_NOTE),
                text_part("reminder")
            ]
        )
    );

    let without_text = message("system", None);
    assert_eq!(
        system_message_as_user(&without_text),
        parts_message("user", vec![text_part(text_parts::CONVERTED_SYSTEM_NOTE)])
    );
}
