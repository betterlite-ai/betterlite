use super::transformation::{config_overrides, toml_key, toml_value};
use crate::types::JsonObject;
use serde_json::Value;

#[test]
#[expect(
    clippy::disallowed_types,
    reason = "The test exercises the untyped JSON configuration boundary"
)]
fn config_translation_covers_toml_and_managed_keys() {
    assert_eq!(toml_key("plain-key"), "plain-key");
    assert_eq!(toml_key("a key"), "\"a key\"");
    let value: Value = serde_json::from_str("{\"a key\": [true, 2]}").unwrap();
    assert_eq!(toml_value(&value, 0).unwrap(), "{\"a key\" = [true, 2]}");
    let config = JsonObject::from([("model".to_owned(), Value::String("wrong".to_owned()))]);
    config_overrides(&config).unwrap_err();
}
