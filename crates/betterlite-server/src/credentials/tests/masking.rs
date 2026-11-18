#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

use super::super::masking::{masked_credential, masked_values};
use super::support::{credential, credential_values, fields, number, payload, text};

fn nested(levels: usize, leaf: JsonData) -> JsonData {
    let mut value = leaf;
    for level in (1..=levels).rev() {
        let name = format!("key_{level}");
        value = JsonData::Object(fields(&[(name.as_str(), value)]));
    }
    value
}

fn nested_arrays(levels: usize, leaf: JsonData) -> JsonData {
    let mut value = leaf;
    for _ in 0..levels {
        value = JsonData::Array(vec![value]);
    }
    value
}

#[test]
fn masking_keeps_the_name_and_info() {
    let masked = masked_credential(&credential(
        "openai",
        &[("team", text("ml"))],
        &[("api_key", text("sk-abcdefgh"))],
    ));
    assert_eq!("openai", masked.credential_name);
    assert_eq!(payload(&[("team", text("ml"))]), masked.credential_info);
    assert_eq!(
        payload(&[("api_key", text("sk****gh"))]),
        masked.credential_values
    );
}

#[test]
fn values_of_four_characters_or_fewer_become_five_asterisks() {
    let masked = masked_values(&credential_values(&[
        ("api_key", text("abcd")),
        ("token", text("")),
        ("password", text("abcde")),
    ]));
    assert_eq!(text("*****"), masked.0["api_key"]);
    assert_eq!(text("*****"), masked.0["token"]);
    assert_eq!(text("ab****de"), masked.0["password"]);
}

#[test]
fn masking_matches_sensitive_keyword_substrings() {
    let masked = masked_values(&credential_values(&[
        ("openai_api_key", text("sk-abcdefgh")),
        ("X-Token", text("secret-token")),
        ("db_password", text("hunter2hunter")),
        ("api_base", text("https://example.test")),
    ]));
    assert_eq!(text("sk****gh"), masked.0["openai_api_key"]);
    assert_eq!(text("se****en"), masked.0["X-Token"]);
    assert_eq!(text("hu****er"), masked.0["db_password"]);
    assert_eq!(text("https://example.test"), masked.0["api_base"]);
}

#[test]
fn masking_recurses_into_an_object_under_a_sensitive_key() {
    let masked = masked_values(&credential_values(&[(
        "vertex_credentials",
        JsonData::Object(fields(&[
            ("token", text("abcdefgh")),
            ("project", text("visible")),
        ])),
    )]));
    let JsonData::Object(inner) = &masked.0["vertex_credentials"] else {
        panic!("the object stays an object");
    };
    assert_eq!(text("ab****gh"), inner["token"]);
    assert_eq!(text("visible"), inner["project"]);
}

#[test]
fn masking_hides_sensitive_keys_under_a_plain_object() {
    let masked = masked_values(&credential_values(&[(
        "vertex_config",
        JsonData::Object(fields(&[
            ("api_key", text("abcdefgh")),
            ("region", text("europe")),
        ])),
    )]));
    let JsonData::Object(inner) = &masked.0["vertex_config"] else {
        panic!("the object stays an object");
    };
    assert_eq!(text("ab****gh"), inner["api_key"]);
    assert_eq!(text("europe"), inner["region"]);
}

#[test]
fn masking_masks_strings_in_an_array_under_a_sensitive_key() {
    let masked = masked_values(&credential_values(&[(
        "api_key",
        JsonData::Array(vec![text("abcdefgh"), text("klmnopqr")]),
    )]));
    assert_eq!(
        JsonData::Array(vec![text("ab****gh"), text("kl****qr")]),
        masked.0["api_key"]
    );
}

#[test]
fn masking_walks_arrays_under_a_plain_key() {
    let masked = masked_values(&credential_values(&[(
        "sources",
        JsonData::Array(vec![
            text("visible"),
            JsonData::Object(fields(&[
                ("token", text("abcdefgh")),
                ("name", text("prod")),
            ])),
        ]),
    )]));
    let JsonData::Array(sources) = &masked.0["sources"] else {
        panic!("the array stays an array");
    };
    assert_eq!(text("visible"), sources[0]);
    let JsonData::Object(source) = &sources[1] else {
        panic!("the source stays an object");
    };
    assert_eq!(text("ab****gh"), source["token"]);
    assert_eq!(text("prod"), source["name"]);
}

#[test]
fn masking_keeps_numbers_and_bools() {
    let masked = masked_values(&credential_values(&[
        ("token_count", number(42)),
        ("secret_flag", JsonData::Bool(true)),
        (
            "password",
            JsonData::Object(fields(&[
                ("port", number(5432)),
                ("enabled", JsonData::Bool(false)),
            ])),
        ),
    ]));
    assert_eq!(number(42), masked.0["token_count"]);
    assert_eq!(JsonData::Bool(true), masked.0["secret_flag"]);
    let JsonData::Object(password) = &masked.0["password"] else {
        panic!("the object stays an object");
    };
    assert_eq!(number(5432), password["port"]);
    assert_eq!(JsonData::Bool(false), password["enabled"]);
}

#[test]
fn masking_redacts_a_container_past_the_depth_limit() {
    let leaf = JsonData::Object(fields(&[("token", text("abcdefgh"))]));
    let deep = masked_values(&credential_values(&[("key", nested(20, leaf.clone()))]));
    assert_eq!(nested(20, text("*****")), deep.0["key"]);
    let shallow = masked_values(&credential_values(&[("key", nested(19, leaf))]));
    let masked_leaf = JsonData::Object(fields(&[("token", text("ab****gh"))]));
    assert_eq!(nested(19, masked_leaf), shallow.0["key"]);
    let arrays = masked_values(&credential_values(&[(
        "key",
        nested_arrays(21, text("abcdefgh")),
    )]));
    assert_eq!(nested_arrays(20, text("*****")), arrays.0["key"]);
}
