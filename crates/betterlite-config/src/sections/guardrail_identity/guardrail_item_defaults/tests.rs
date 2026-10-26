#![cfg(test)]

use super::super::enums::Role;
use super::super::item::GuardrailItem;

#[test]
fn omitted_roles_get_all_roles() {
    let item = GuardrailItem::new(Vec::new(), String::from("presidio"));
    assert_eq!(
        item.enabled_roles,
        Some(vec![Role::System, Role::Assistant, Role::User])
    );
    assert!(!item.default_on);
    assert!(item.callback_args.is_empty());
}
