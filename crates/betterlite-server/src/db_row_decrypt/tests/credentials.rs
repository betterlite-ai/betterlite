use std::collections::BTreeMap;

use betterlite_config::credentials::{CredentialItem, CredentialValue};
use betterlite_db::budget_permission_rows::JsonData;
use serde_json::Number;

use super::support::{cipher, config, credential, sealed};
use crate::db_row_decrypt::credentials::{credential_list, decrypt_credential};

fn value<'item>(item: &'item CredentialItem, name: &str) -> &'item CredentialValue {
    &item.credential_values.0[name]
}

#[test]
fn decrypts_credential_text_values() {
    let cipher = cipher();
    let item = credential(&format!(
        r#"{{"credential_name":"c","credential_info":{{}},"credential_values":{{"api_key":"{}","api_base":"https://api.test"}}}}"#,
        sealed(&cipher, "sk-cred")
    ));
    let decrypted = decrypt_credential(&item, &cipher);
    assert_eq!(
        value(&decrypted, "api_key").expose_json(),
        JsonData::String("sk-cred".to_owned())
    );
    assert_eq!(
        value(&decrypted, "api_base").expose_json(),
        JsonData::String("https://api.test".to_owned())
    );
    assert_eq!(decrypted.credential_name, "c");
}

#[test]
fn keeps_stored_text_when_decryption_gives_nothing() {
    let cipher = cipher();
    let item = credential(
        r#"{"credential_name":"c","credential_info":{},"credential_values":{"broken":"not-a-ciphertext"}}"#,
    );
    let decrypted = decrypt_credential(&item, &cipher);
    assert_eq!(
        value(&decrypted, "broken").expose_json(),
        JsonData::String("not-a-ciphertext".to_owned())
    );
}

#[test]
fn uses_an_empty_plaintext_as_given() {
    let cipher = cipher();
    let empty = sealed(&cipher, "");
    let item = credential(&format!(
        r#"{{"credential_name":"c","credential_info":{{}},"credential_values":{{"empty":"{empty}"}}}}"#
    ));
    let decrypted = decrypt_credential(&item, &cipher);
    assert_eq!(
        value(&decrypted, "empty").expose_json(),
        JsonData::String(String::new())
    );
}

#[test]
fn leaves_non_text_values_alone() {
    let cipher = cipher();
    let item = credential(
        r#"{"credential_name":"c","credential_info":{},"credential_values":{"port":6379,"flag":true,"nothing":null,"list":["a"],"object":{"k":"v"}}}"#,
    );
    let decrypted = decrypt_credential(&item, &cipher);
    assert_eq!(
        value(&decrypted, "port").expose_json(),
        JsonData::Number(Number::from(6379_i64))
    );
    assert_eq!(
        value(&decrypted, "flag").expose_json(),
        JsonData::Bool(true)
    );
    assert_eq!(value(&decrypted, "nothing").expose_json(), JsonData::Null);
    assert_eq!(
        value(&decrypted, "list").expose_json(),
        JsonData::Array(vec![JsonData::String("a".to_owned())])
    );
    assert_eq!(
        value(&decrypted, "object").expose_json(),
        JsonData::Object(BTreeMap::from([(
            "k".to_owned(),
            JsonData::String("v".to_owned())
        )]))
    );
}

#[test]
fn credential_list_reads_the_config_section() {
    let configured = config(
        "credential_list:\n  - credential_name: c\n    credential_info: {}\n    credential_values:\n      api_key: sk-1\n",
    );
    let items = credential_list(&configured);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].credential_name, "c");
    let empty = config("model_list: []\n");
    assert!(credential_list(&empty).is_empty());
}
