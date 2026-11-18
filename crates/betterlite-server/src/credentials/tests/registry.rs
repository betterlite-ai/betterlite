use super::support::{credential, payload, registry, text};

#[test]
fn first_matching_credential_wins() {
    let registry = registry(vec![
        credential("openai", &[], &[("api_key", text("first"))]),
        credential("openai", &[], &[("api_key", text("second"))]),
    ]);
    let found = registry
        .find_credential("openai")
        .expect("the credential is registered");
    assert_eq!(
        payload(&[("api_key", text("first"))]),
        found.fields.credential_values
    );
    assert_eq!(
        payload(&[("api_key", text("first"))]),
        registry.get_credential_values("openai")
    );
}

#[test]
fn unknown_credentials_have_no_values() {
    let registry = registry(Vec::new());
    assert!(registry.find_credential("missing").is_none());
    assert!(registry.get_credential_values("missing").0.is_empty());
}

#[test]
fn an_upsert_replaces_the_first_match_in_place() {
    let registry = registry(vec![
        credential("a", &[], &[("api_key", text("old-a"))]),
        credential("b", &[], &[("api_key", text("b"))]),
    ]);
    registry.upsert_credentials(vec![credential("a", &[], &[("api_key", text("new-a"))])]);
    let listed = registry.credential_list();
    assert_eq!(2, listed.len());
    assert_eq!("a", listed[0].fields.base.credential_name);
    assert_eq!(
        payload(&[("api_key", text("new-a"))]),
        listed[0].fields.credential_values
    );
    assert_eq!("b", listed[1].fields.base.credential_name);
}

#[test]
fn two_new_credentials_with_one_name_both_append() {
    let registry = registry(Vec::new());
    registry.upsert_credentials(vec![
        credential("dup", &[], &[("api_key", text("first"))]),
        credential("dup", &[], &[("api_key", text("second"))]),
    ]);
    assert_eq!(2, registry.credential_list().len());
    assert_eq!(
        payload(&[("api_key", text("first"))]),
        registry.get_credential_values("dup")
    );
}

#[test]
fn removal_drops_every_entry_with_the_name() {
    let registry = registry(vec![
        credential("dup", &[], &[]),
        credential("kept", &[], &[]),
        credential("dup", &[], &[]),
    ]);
    registry.remove_credential("dup");
    let names: Vec<String> = registry
        .credential_list()
        .into_iter()
        .map(|credential| credential.fields.base.credential_name)
        .collect();
    assert_eq!(vec!["kept".to_owned()], names);
}

#[test]
fn the_credential_list_is_a_snapshot() {
    let registry = registry(vec![credential("a", &[], &[])]);
    let mut listed = registry.credential_list();
    listed.clear();
    assert_eq!(1, registry.credential_list().len());
}
