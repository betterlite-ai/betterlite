pub mod transformation;

#[cfg(test)]
mod codex_transformation {
    use super::transformation::{config_overrides, toml_key, toml_value};
    use crate::types::JsonObject;
    use serde_json::json;

    #[test]
    fn config_translation_covers_toml_and_managed_keys() {
        assert_eq!(toml_key("plain-key"), "plain-key");
        assert_eq!(toml_key("a key"), "\"a key\"");
        assert_eq!(
            toml_value(&json!({"a key": [true, 2]}), 0).unwrap(),
            "{\"a key\" = [true, 2]}"
        );
        let config = JsonObject::from([("model".to_owned(), json!("wrong"))]);
        assert!(config_overrides(&config).is_err());
    }
}
