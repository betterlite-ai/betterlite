use betterlite_db::budget_permission_rows::JsonData;

use super::super::masking::{mask_fields, masked_credential};
use super::support::{credential, fields, number, payload, text};

fn nested(levels: usize, leaf: JsonData) -> JsonData {
    let mut value = leaf;
    for level in (1..=levels).rev() {
        let name = format!("key_{level}");
        value = JsonData::Object(fields(&[(name.as_str(), value)]));
    }
    value
}

fn follow(mut value: &JsonData, levels: usize) -> &JsonData {
    for level in 1..=levels {
        let JsonData::Object(inner) = value else {
            panic!("the chain holds objects");
        };
        value = &inner[&format!("key_{level}")];
    }
    value
}

#[test]
fn masking_keeps_the_name_and_info() {
    let masked = masked_credential(credential(
        "openai",
        &[("team", text("ml"))],
        &[("api_key", text("sk-secret-value"))],
    ));
    assert_eq!("openai", masked.fields.base.credential_name);
    assert_eq!(
        payload(&[("team", text("ml"))]),
        masked.fields.base.credential_info
    );
    assert_eq!(
        payload(&[("api_key", text("sk****ue"))]),
        masked.fields.credential_values
    );
}

#[test]
fn values_of_four_characters_or_fewer_become_five_asterisks() {
    let masked = mask_fields(
        &fields(&[
            ("api_key", text("abcd")),
            ("token", text("")),
            ("password", text("abcde")),
        ]),
        0,
    );
    assert_eq!(text("*****"), masked["api_key"]);
    assert_eq!(text("*****"), masked["token"]);
    assert_eq!(text("ab****de"), masked["password"]);
}

#[test]
fn masking_matches_sensitive_keyword_substrings() {
    let masked = mask_fields(
        &fields(&[
            ("openai_api_key", text("sk-abcdefgh")),
            ("X-Token", text("secret-token")),
            ("db_password", text("hunter2hunter")),
            ("api_base", text("https://example.test")),
        ]),
        0,
    );
    assert_eq!(text("sk****gh"), masked["openai_api_key"]);
    assert_eq!(text("se****en"), masked["X-Token"]);
    assert_eq!(text("hu****er"), masked["db_password"]);
    assert_eq!(text("https://example.test"), masked["api_base"]);
}

#[test]
fn masking_recurses_into_an_object_under_a_sensitive_key() {
    let masked = mask_fields(
        &fields(&[(
            "vertex_credentials",
            JsonData::Object(fields(&[
                ("token", text("abcdefgh")),
                ("project", text("visible")),
            ])),
        )]),
        0,
    );
    let JsonData::Object(inner) = &masked["vertex_credentials"] else {
        panic!("the object stays an object");
    };
    assert_eq!(text("ab****gh"), inner["token"]);
    assert_eq!(text("visible"), inner["project"]);
}

#[test]
fn masking_leaves_an_object_under_a_plain_key_alone() {
    let masked = mask_fields(
        &fields(&[(
            "vertex_config",
            JsonData::Object(fields(&[("api_key", text("abcdefgh"))])),
        )]),
        0,
    );
    let JsonData::Object(inner) = &masked["vertex_config"] else {
        panic!("the object stays an object");
    };
    assert_eq!(text("abcdefgh"), inner["api_key"]);
}

#[test]
fn masking_leaves_arrays_numbers_and_bools_alone() {
    let masked = mask_fields(
        &fields(&[
            ("api_key", JsonData::Array(vec![text("abcdefgh")])),
            ("token_count", number(42)),
            ("secret_flag", JsonData::Bool(true)),
        ]),
        0,
    );
    assert_eq!(JsonData::Array(vec![text("abcdefgh")]), masked["api_key"]);
    assert_eq!(number(42), masked["token_count"]);
    assert_eq!(JsonData::Bool(true), masked["secret_flag"]);
}

#[test]
fn masking_stops_at_the_twentieth_nested_object() {
    let leaf = JsonData::Object(fields(&[("token", text("abcdefgh"))]));
    let deep = mask_fields(&fields(&[("key", nested(20, leaf.clone()))]), 0);
    let JsonData::Object(reached) = follow(&deep["key"], 20) else {
        panic!("the leaf is an object");
    };
    assert_eq!(text("abcdefgh"), reached["token"]);
    let shallow = mask_fields(&fields(&[("key", nested(19, leaf))]), 0);
    let JsonData::Object(reached) = follow(&shallow["key"], 19) else {
        panic!("the leaf is an object");
    };
    assert_eq!(text("ab****gh"), reached["token"]);
}
