#![cfg(test)]

use std::sync::Arc;

use super::super::registry::CredentialRegistry;
use super::support::{credential, credential_values, payload, registry, text};

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
        credential_values(&[("api_key", text("first"))]),
        found.credential_values
    );
    assert_eq!(
        credential_values(&[("api_key", text("first"))]),
        registry.get_credential_values("openai")
    );
}

#[test]
fn lookup_skips_unrelated_names() {
    let registry = registry(vec![
        credential("first", &[], &[("key", text("one"))]),
        credential("second", &[], &[("key", text("two"))]),
    ]);
    assert_eq!(
        registry.get_credential_values("second"),
        credential_values(&[("key", text("two"))])
    );
    assert!(registry.find_credential("missing").is_none());
    assert!(registry.get_credential_values("missing").0.is_empty());
}

#[test]
fn upsert_skips_unrelated_names_and_keeps_later_duplicates() {
    let first = credential("first", &[], &[]);
    let second = credential("second", &[], &[("key", text("old"))]);
    let changed = credential("second", &[], &[("key", text("new"))]);
    let registry = registry(vec![first.clone(), second.clone(), second.clone()]);
    registry.upsert_credentials(vec![changed.clone()]);
    assert_eq!(registry.credential_list(), vec![first, changed, second]);
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
    assert_eq!("a", listed[0].credential_name);
    assert_eq!(
        credential_values(&[("api_key", text("new-a"))]),
        listed[0].credential_values
    );
    assert_eq!("b", listed[1].credential_name);
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
        credential_values(&[("api_key", text("first"))]),
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
        .map(|credential| credential.credential_name)
        .collect();
    assert_eq!(vec!["kept".to_owned()], names);
}

#[test]
fn debug_output_redacts_credential_values() {
    let registry = registry(vec![credential(
        "openai",
        &[],
        &[("api_key", text("sk-abcdefgh"))],
    )]);
    let listed = registry.credential_list();
    let printed = format!("{listed:?}");
    assert!(printed.contains("REDACTED"), "{printed}");
    assert!(!printed.contains("sk-abcdefgh"), "{printed}");
}

#[test]
fn the_credential_list_is_a_snapshot() {
    let registry = registry(vec![credential("a", &[], &[])]);
    let mut listed = registry.credential_list();
    assert_eq!(1, listed.len());
    listed.clear();
    assert_eq!(1, registry.credential_list().len());
}

#[test]
fn a_patch_merges_into_the_resident_credential() {
    let registry = registry(vec![credential(
        "openai",
        &[("team", text("ml"))],
        &[("api_base", text("https://a"))],
    )]);
    let patch = credential(
        "openai",
        &[("owner", text("x"))],
        &[("api_key", text("new-secret"))],
    );
    registry.apply_credential_patch("openai", "openai", &patch);
    assert_eq!(
        credential_values(&[
            ("api_key", text("new-secret")),
            ("api_base", text("https://a")),
        ]),
        registry.get_credential_values("openai")
    );
    let resident = registry.find_credential("openai").expect("the entry stays");
    assert_eq!(
        payload(&[("team", text("ml")), ("owner", text("x"))]),
        resident.credential_info
    );
}

const WORKERS: usize = 8;
const VALUES_PER_WORKER: usize = 25;

fn patch_worker(registry: &CredentialRegistry, worker: usize) {
    for index in 0..VALUES_PER_WORKER {
        let key = format!("key_{worker}_{index}");
        let patch = credential("openai", &[], &[(key.as_str(), text("value"))]);
        registry.apply_credential_patch("openai", "openai", &patch);
    }
}

fn run_workers(registry: &Arc<CredentialRegistry>) {
    std::thread::scope(|scope| {
        for worker in 0..WORKERS {
            let shared = Arc::clone(registry);
            scope.spawn(move || patch_worker(&shared, worker));
        }
    });
}

#[test]
fn a_patch_applies_under_one_lock_so_no_value_is_lost() {
    let registry = registry(vec![credential("openai", &[], &[])]);
    run_workers(&registry);
    let values = registry.get_credential_values("openai");
    assert_eq!(WORKERS * VALUES_PER_WORKER, values.0.len());
}

#[test]
fn rename_preserves_order_and_replaces_only_the_first_target() {
    let registry = registry(vec![
        credential("before", &[], &[]),
        credential("old", &[], &[("key", text("original"))]),
        credential("new", &[], &[("key", text("target"))]),
        credential("kept", &[], &[]),
        credential("old", &[], &[("key", text("duplicate"))]),
        credential("new", &[], &[("key", text("second"))]),
    ]);
    registry.apply_credential_patch("old", "new", &credential("new", &[], &[]));
    assert_eq!(
        registry.credential_list(),
        vec![
            credential("before", &[], &[]),
            credential("new", &[], &[("key", text("original"))]),
            credential("kept", &[], &[]),
            credential("new", &[], &[("key", text("second"))]),
        ]
    );
}
