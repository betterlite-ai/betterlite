use std::collections::BTreeMap;

use betterlite_config::sections::settings::{FieldSource, JsonValue};

use crate::settings_services::projection::settings_with_schema;
use crate::settings_services::response::SsoSettingsResponse;

use super::support::{as_json, json, record, values};

// SSOSettingsResponse: the common values/field_schema pair with each stored field's provenance beside it
#[test]
fn sso_settings_flat_the_common_response_beside_its_provenance() {
    let settings = record(values(&[("user_role", json!("internal_user_viewer"))]));
    let response = SsoSettingsResponse {
        settings: settings_with_schema(&settings),
        provenance: BTreeMap::from([("user_role".to_owned(), FieldSource::Db)]),
    };
    assert_eq!(
        as_json(&response),
        json!({
            "values": {"user_role": "internal_user_viewer"},
            "field_schema": {"description": "", "properties": {}},
            "provenance": {"user_role": "db"}
        })
    );
}
