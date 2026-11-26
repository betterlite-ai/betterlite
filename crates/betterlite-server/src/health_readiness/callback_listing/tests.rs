use betterlite_logging::registry::identity::CallbackIdentity;
use indexmap::IndexMap;

use super::{CallbackEntry, callback_name};

#[test]
fn callbacks_callback_name_returns_a_plain_name() {
    let entry = CallbackEntry::Plain("langfuse".to_owned());
    assert_eq!(callback_name(&entry), "langfuse");
}

#[test]
fn callbacks_callback_name_returns_the_registered_class_name() {
    let identity = CallbackIdentity {
        class_name: "MyCustomLogger".to_owned(),
        attributes: IndexMap::new(),
    };
    let entry = CallbackEntry::Registered(identity);
    assert_eq!(callback_name(&entry), "MyCustomLogger");
}
