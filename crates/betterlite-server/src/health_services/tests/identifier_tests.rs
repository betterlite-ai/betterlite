use betterlite_logging::registry::identity::CallbackIdentity;
use indexmap::IndexMap;

use crate::health_readiness::callback_listing::CallbackEntry;
use crate::health_services::callback_id::get_callback_identifier;

#[test]
fn identifier_plain_entry_is_its_own_name() {
    let entry = CallbackEntry::Plain("datadog".to_owned());
    assert_eq!(get_callback_identifier(&entry), "datadog");
}

#[test]
fn identifier_registered_entry_is_its_registered_name() {
    let entry = CallbackEntry::Registered(CallbackIdentity {
        class_name: "langfuse".to_owned(),
        attributes: IndexMap::new(),
    });
    assert_eq!(get_callback_identifier(&entry), "langfuse");
}
