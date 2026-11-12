use crate::database_refresh::handle::{PoolProvider as _, PoolTransport};
use crate::database_refresh::native::{NativePoolError, NativePoolProvider};

use super::support::secret;

#[test]
fn native_pool_build_honors_transport() {
    let provider = NativePoolProvider;
    let url = secret("postgresql://user:password@127.0.0.1:5432/database");
    let transport = PoolTransport { max_size: Some(3) };
    let tuned = provider
        .build(&url, Some(&transport))
        .expect("the tuned pool builds");
    assert_eq!(tuned.pool.status().max_size, 3);
    let defaulted = provider.build(&url, None).expect("the default pool builds");
    assert_ne!(defaulted.pool.status().max_size, 3);
}

#[test]
fn native_pool_build_rejects_invalid_url() {
    let provider = NativePoolProvider;
    let error = provider
        .build(&secret("not a database url"), None)
        .err()
        .expect("an invalid URL fails");
    assert!(matches!(error, NativePoolError::Options { .. }));
}
