use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

mod http;
mod reader;
mod socket;
mod tls;

use super::support::{assert_secret, empty_environment, sync_reader_fake, vault_client};

#[test]
fn vault_returns_the_secret() {
    let fake = sync_reader_fake(Some("vault-value"));
    let client = vault_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::HashicorpVault),
        "kv/myapp/config",
        None,
        &empty_environment(),
    );
    assert_secret(result, "vault-value");
    assert_eq!(fake.names(), ["kv/myapp/config"]);
}
