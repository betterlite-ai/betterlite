#![cfg(test)]

use super::*;
use crate::arguments::json::Object;

fn message(role: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: None,
        name: None,
        provider_specific_fields: Object::new(),
    }
}

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_a_user_role_message_has_its_own_kind();
}

// A user-role message carries MessageKind::User, distinct from the Other fallback every unmatched role falls into.
fn assert_a_user_role_message_has_its_own_kind() {
    assert_eq!(kind(&message("user")), MessageKind::User);
}
