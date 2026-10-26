#![cfg(test)]

use super::super::enums::Role;
use super::super::item::{CallbackArgs, GuardrailItem};

#[test]
fn omitted_roles_get_all_roles() {
    let item = GuardrailItem {
        callbacks: Vec::new(),
        default_on: false,
        logging_only: None,
        guardrail_name: String::from("presidio"),
        callback_args: CallbackArgs::new(),
        enabled_roles: Some(super::super::enums::default_roles()),
    };
    assert_eq!(
        item.enabled_roles,
        Some(vec![Role::System, Role::Assistant, Role::User])
    );
    assert!(!item.default_on);
    assert!(item.callback_args.is_empty());
}
