use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

use super::support::{assert_secret, cyberark_client, empty_environment, sync_reader_fake};

#[test]
fn cyberark_returns_the_secret() {
    let fake = sync_reader_fake(Some("cyberark-value"));
    let client = cyberark_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::Cyberark),
        "db/password",
        None,
        &empty_environment(),
    );
    assert_secret(result, "cyberark-value");
    assert_eq!(fake.names(), ["db/password"]);
}
