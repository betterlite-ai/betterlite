#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{ClaimAttributes, load_claim_attributes};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

fn defaults() -> ClaimAttributes {
    ClaimAttributes {
        user_id: "preferred_username".to_owned(),
        display_name: "sub".to_owned(),
        email: "email".to_owned(),
        first_name: "first_name".to_owned(),
        last_name: "last_name".to_owned(),
        provider: "provider".to_owned(),
        role: "role".to_owned(),
        extra_attributes: Vec::new(),
    }
}

#[rstest]
#[case::nothing_set(&[], defaults())]
#[case::id_binding(
    &[("GENERIC_USER_ID_ATTRIBUTE", "uid")],
    ClaimAttributes { user_id: "uid".to_owned(), ..defaults() }
)]
#[case::display_name_binding(
    &[("GENERIC_USER_DISPLAY_NAME_ATTRIBUTE", "name")],
    ClaimAttributes { display_name: "name".to_owned(), ..defaults() }
)]
#[case::email_binding(
    &[("GENERIC_USER_EMAIL_ATTRIBUTE", "mail")],
    ClaimAttributes { email: "mail".to_owned(), ..defaults() }
)]
#[case::first_name_binding(
    &[("GENERIC_USER_FIRST_NAME_ATTRIBUTE", "given_name")],
    ClaimAttributes { first_name: "given_name".to_owned(), ..defaults() }
)]
#[case::last_name_binding(
    &[("GENERIC_USER_LAST_NAME_ATTRIBUTE", "family_name")],
    ClaimAttributes { last_name: "family_name".to_owned(), ..defaults() }
)]
#[case::provider_binding(
    &[("GENERIC_USER_PROVIDER_ATTRIBUTE", "idp")],
    ClaimAttributes { provider: "idp".to_owned(), ..defaults() }
)]
#[case::role_binding(
    &[("GENERIC_USER_ROLE_ATTRIBUTE", "custom_roles")],
    ClaimAttributes { role: "custom_roles".to_owned(), ..defaults() }
)]
#[case::empty_binding_takes_the_default(
    &[("GENERIC_USER_ID_ATTRIBUTE", "")],
    defaults()
)]
#[case::space_binding_is_a_path(
    &[("GENERIC_USER_ID_ATTRIBUTE", " ")],
    ClaimAttributes { user_id: " ".to_owned(), ..defaults() }
)]
#[case::extra_attributes_empty_is_absent(
    &[("GENERIC_USER_EXTRA_ATTRIBUTES", "")],
    defaults()
)]
#[case::extra_attributes_single(
    &[("GENERIC_USER_EXTRA_ATTRIBUTES", "tenant.id")],
    ClaimAttributes { extra_attributes: vec!["tenant.id".to_owned()], ..defaults() }
)]
#[case::extra_attributes_split_and_trim(
    &[("GENERIC_USER_EXTRA_ATTRIBUTES", " a.b , c ,nested\\.key")],
    ClaimAttributes {
        extra_attributes: vec!["a.b".to_owned(), "c".to_owned(), "nested\\.key".to_owned()],
        ..defaults()
    }
)]
#[case::extra_attributes_keep_an_empty_name(
    &[("GENERIC_USER_EXTRA_ATTRIBUTES", "a,")],
    ClaimAttributes { extra_attributes: vec!["a".to_owned(), String::new()], ..defaults() }
)]
#[case::extra_attributes_space_only_names_the_empty_path(
    &[("GENERIC_USER_EXTRA_ATTRIBUTES", " ")],
    ClaimAttributes { extra_attributes: vec![String::new()], ..defaults() }
)]
#[case::every_binding(
    &[
        ("GENERIC_USER_ID_ATTRIBUTE", "uid"),
        ("GENERIC_USER_DISPLAY_NAME_ATTRIBUTE", "name"),
        ("GENERIC_USER_EMAIL_ATTRIBUTE", "mail"),
        ("GENERIC_USER_FIRST_NAME_ATTRIBUTE", "given_name"),
        ("GENERIC_USER_LAST_NAME_ATTRIBUTE", "family_name"),
        ("GENERIC_USER_PROVIDER_ATTRIBUTE", "idp"),
        ("GENERIC_USER_ROLE_ATTRIBUTE", "custom_roles"),
        ("GENERIC_USER_EXTRA_ATTRIBUTES", "tenant"),
    ],
    ClaimAttributes {
        user_id: "uid".to_owned(),
        display_name: "name".to_owned(),
        email: "mail".to_owned(),
        first_name: "given_name".to_owned(),
        last_name: "family_name".to_owned(),
        provider: "idp".to_owned(),
        role: "custom_roles".to_owned(),
        extra_attributes: vec!["tenant".to_owned()],
    }
)]
fn loads_claim_attributes(#[case] pairs: &[(&str, &str)], #[case] expected: ClaimAttributes) {
    assert_eq!(load_claim_attributes(&environment(pairs)), expected);
}
