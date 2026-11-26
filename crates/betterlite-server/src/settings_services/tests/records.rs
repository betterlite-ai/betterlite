use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

use crate::settings_services::records::SettingsRecord;
use crate::settings_services::schema::RootSchema;

use super::support::{json, values};

// A record whose class declares one default and leaves the other field without one, the two cases _model_field_default tells apart
struct DeclaredDefaults {
    schema: RootSchema,
    defaults: BTreeMap<String, JsonValue>,
}

impl SettingsRecord for DeclaredDefaults {
    fn schema(&self) -> &RootSchema {
        &self.schema
    }

    fn values(&self) -> BTreeMap<String, JsonValue> {
        self.defaults.clone()
    }

    fn field_default(&self, field_name: &str) -> Option<JsonValue> {
        self.defaults.get(field_name).cloned()
    }
}

#[test]
fn field_default_reports_the_declared_default_and_none_otherwise() {
    let record = DeclaredDefaults {
        schema: RootSchema::default(),
        defaults: values(&[("user_role", json!("internal_user_viewer"))]),
    };
    assert_eq!(
        record.field_default("user_role"),
        Some(json!("internal_user_viewer"))
    );
    assert_eq!(record.field_default("max_budget"), None);
}
