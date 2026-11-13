use secrecy::ExposeSecret as _;
use tokio::sync::Mutex;

use betterlite_secrets::gateway_key::{
    GatewayKeyStore, StoredCredential, get_litellm_gateway_api_key,
};

use crate::credential_lifecycle::gateway::VaultGatewayStore;
use crate::token_records::secret::decode_secret;

use super::support::{
    OTHER_SERVER, SERVER, fake_vault, metadata, record, scripted_files, secret, vault_entry,
};

struct FixedStore(Option<StoredCredential>);

impl GatewayKeyStore for FixedStore {
    fn stored_credential(&self, _expected_base_url: Option<&str>) -> Option<StoredCredential> {
        self.0.clone()
    }
}

fn keyed() -> FixedStore {
    FixedStore(Some(StoredCredential {
        base_url: SERVER.to_owned(),
        key: Some(secret("sk-secret")),
    }))
}

#[test]
fn gateway_key_is_none_without_a_stored_record() {
    assert!(get_litellm_gateway_api_key(None, &FixedStore(None)).is_none());
    assert!(get_litellm_gateway_api_key(Some(SERVER), &FixedStore(None)).is_none());
}

#[test]
fn gateway_key_is_none_when_the_origin_differs() {
    assert!(get_litellm_gateway_api_key(Some(OTHER_SERVER), &keyed()).is_none());
}

#[test]
fn gateway_key_strips_the_trailing_slashes_of_the_expected_origin() {
    assert_eq!(
        get_litellm_gateway_api_key(Some("https://gateway.example///"), &keyed()),
        Some("sk-secret".to_owned())
    );
}

#[test]
fn gateway_key_returns_the_key_of_a_matching_origin() {
    assert_eq!(
        get_litellm_gateway_api_key(Some(SERVER), &keyed()),
        Some("sk-secret".to_owned())
    );
    assert_eq!(
        get_litellm_gateway_api_key(None, &keyed()),
        Some("sk-secret".to_owned())
    );
}

#[test]
fn gateway_key_is_none_when_the_store_resolved_no_key() {
    let store = FixedStore(Some(StoredCredential {
        base_url: SERVER.to_owned(),
        key: None,
    }));
    assert!(get_litellm_gateway_api_key(Some(SERVER), &store).is_none());
}

#[test]
fn vault_store_returns_the_keychain_key_for_a_matching_origin() {
    let files = scripted_files();
    files.put(&metadata(2_000.0));
    let mut initial = fake_vault();
    initial.blob = Some(vault_entry(SERVER, "sk-vault", None, 2_000.0));
    let vault = Mutex::new(initial);
    let key = {
        let store = VaultGatewayStore {
            files: &files,
            vault: &vault,
        };
        get_litellm_gateway_api_key(Some(SERVER), &store)
    };
    assert_eq!(key, Some("sk-vault".to_owned()));
    assert!(vault.into_inner().writes().is_empty());
}

#[test]
fn vault_store_does_not_read_the_keychain_of_a_foreign_origin() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let vault = Mutex::new(fake_vault());
    let key = {
        let store = VaultGatewayStore {
            files: &files,
            vault: &vault,
        };
        get_litellm_gateway_api_key(Some(OTHER_SERVER), &store)
    };
    assert!(key.is_none());
    let observed = vault.into_inner();
    assert_eq!(observed.reads(), 0);
    assert!(observed.writes().is_empty());
}

#[test]
fn vault_store_migrates_a_file_secret_on_a_matching_origin() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let vault = Mutex::new(fake_vault());
    let key = {
        let store = VaultGatewayStore {
            files: &files,
            vault: &vault,
        };
        get_litellm_gateway_api_key(Some(SERVER), &store)
    };
    assert_eq!(key, Some("sk-secret".to_owned()));
    let observed = vault.into_inner();
    assert_eq!(observed.writes().len(), 1);
    let held = observed.blob.as_ref().expect("the vault holds the secret");
    let decoded = decode_secret(held.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(decoded.key.expect("a key").expose_secret(), "sk-secret");
    assert!(files.stored().expect("the file stays").key.is_none());
}

#[test]
fn vault_store_leaves_the_keychain_secret_for_the_next_client() {
    let files = scripted_files();
    files.put(&metadata(2_000.0));
    let mut initial = fake_vault();
    initial.blob = Some(vault_entry(SERVER, "sk-vault", None, 2_000.0));
    let vault = Mutex::new(initial);
    let (mismatch, matching) = {
        let store = VaultGatewayStore {
            files: &files,
            vault: &vault,
        };
        (
            get_litellm_gateway_api_key(Some(OTHER_SERVER), &store),
            get_litellm_gateway_api_key(Some(SERVER), &store),
        )
    };
    assert!(mismatch.is_none());
    assert_eq!(matching, Some("sk-vault".to_owned()));
    let observed = vault.into_inner();
    assert_eq!(observed.reads(), 1);
    assert!(observed.writes().is_empty());
    assert!(observed.blob.is_some());
}

#[test]
fn vault_store_returns_none_when_a_metadata_only_file_has_no_keychain_entry() {
    let files = scripted_files();
    files.put(&metadata(1_000.0));
    let vault = Mutex::new(fake_vault());
    let key = {
        let store = VaultGatewayStore {
            files: &files,
            vault: &vault,
        };
        get_litellm_gateway_api_key(Some(SERVER), &store)
    };
    assert!(key.is_none());
}
