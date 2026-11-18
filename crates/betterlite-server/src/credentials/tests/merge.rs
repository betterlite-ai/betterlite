#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::credential::CredentialItem as CredentialRow;

use super::super::error::CredentialError;
use super::super::merge::{CredentialUpdate, merged_credential, update_db_credential};
use super::support::{
    FakeStore, StoreFailure, USER_ID, cipher, credential, decrypt_text, payload, row, sealed, text,
};

fn stored() -> CredentialRow {
    row(&credential(
        "openai",
        &[("team", text("ml"))],
        &[
            ("api_key", text(&sealed("old-secret"))),
            ("api_base", text("https://a")),
        ],
    ))
}

#[test]
fn update_merges_the_supplied_values_into_the_stored_ones() {
    let existing = stored();
    let patch = credential("", &[], &[("api_key", text("new-secret"))]);
    let merged = merged_credential(&cipher(), &existing, &patch, None).expect("the patch merges");
    let values = &merged.fields.credential_values.0;
    assert_eq!(text("https://a"), values["api_base"]);
    let JsonData::String(sealed) = &values["api_key"] else {
        panic!("a sealed value is text");
    };
    assert_eq!("new-secret", decrypt_text(sealed));
}

#[test]
fn update_keeps_the_stored_name_when_the_patch_name_is_empty() {
    let existing = stored();
    let patch = credential("", &[], &[("api_key", text("new-secret"))]);
    let merged = merged_credential(&cipher(), &existing, &patch, None).expect("the patch merges");
    assert_eq!("openai", merged.fields.base.credential_name);
}

#[test]
fn update_takes_the_patch_name_when_it_is_set() {
    let existing = stored();
    let patch = credential("renamed", &[], &[("api_key", text("new-secret"))]);
    let merged = merged_credential(&cipher(), &existing, &patch, None).expect("the patch merges");
    assert_eq!("renamed", merged.fields.base.credential_name);
}

#[test]
fn update_drops_the_old_info_unless_it_carries_the_literal_key() {
    let existing = stored();
    let patch = credential("", &[("owner", text("x"))], &[]);
    let merged = merged_credential(&cipher(), &existing, &patch, None).expect("the patch merges");
    assert_eq!(
        payload(&[("owner", text("x"))]),
        merged.fields.base.credential_info
    );
}

#[test]
fn update_keeps_the_old_info_that_carries_the_literal_key() {
    let mut existing = stored();
    existing
        .fields
        .base
        .credential_info
        .0
        .insert("credential_info".to_owned(), JsonData::Bool(true));
    let patch = credential("", &[("owner", text("x"))], &[]);
    let merged = merged_credential(&cipher(), &existing, &patch, None).expect("the patch merges");
    assert_eq!(
        payload(&[
            ("credential_info", JsonData::Bool(true)),
            ("team", text("ml")),
            ("owner", text("x")),
        ]),
        merged.fields.base.credential_info
    );
}

#[test]
fn update_leaves_values_and_info_alone_for_an_empty_patch() {
    let existing = stored();
    let patch = credential("", &[], &[]);
    let merged = merged_credential(&cipher(), &existing, &patch, None).expect("the patch merges");
    assert_eq!(existing, merged);
}

#[tokio::test]
async fn update_writes_under_the_original_name_with_the_actor() {
    let store = FakeStore::seeded(vec![stored()]);
    let existing = store.row("openai").expect("the row is seeded");
    let patch = credential("renamed", &[], &[("api_key", text("new-secret"))]);
    let merged = update_db_credential(
        store.as_ref(),
        &cipher(),
        &existing,
        CredentialUpdate {
            patch: &patch,
            new_encryption_key: None,
            actor: Some(USER_ID),
        },
    )
    .await
    .expect("the update writes");
    let writes = store.updated();
    assert_eq!(1, writes.len());
    assert_eq!("openai", writes[0].0);
    assert_eq!(Some(USER_ID.to_owned()), writes[0].2);
    assert_eq!("renamed", writes[0].1.fields.base.credential_name);
    assert_eq!("renamed", merged.fields.base.credential_name);
    assert!(store.row("openai").is_none());
    assert!(store.row("renamed").is_some());
}

#[tokio::test]
async fn a_failed_store_write_surfaces_an_error() {
    let store = FakeStore::seeded(vec![stored()]);
    store.fail(StoreFailure::Update);
    let existing = store.row("openai").expect("the row is seeded");
    let patch = credential("", &[], &[("api_key", text("new-secret"))]);
    let result = update_db_credential(
        store.as_ref(),
        &cipher(),
        &existing,
        CredentialUpdate {
            patch: &patch,
            new_encryption_key: None,
            actor: None,
        },
    )
    .await;
    assert!(matches!(result, Err(CredentialError::Store { .. })));
    assert!(store.updated().is_empty());
}
