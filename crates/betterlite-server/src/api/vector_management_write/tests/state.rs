#![cfg(test)]

use super::support::{Setup, memory_store, read_state};
use crate::api::vector_management_write::state::VectorManagementWriteState;

#[test]
fn write_state_takes_the_cache_from_the_read_registry() {
    let mut setup = Setup::default();
    setup.registry = Some(vec![memory_store("vs_a")]);
    let state = VectorManagementWriteState::from(read_state(setup));
    let cache = state
        .cache
        .expect("the read registry becomes the write cache");
    let found = cache.find("vs_a").expect("the cache carries the store");
    assert_eq!(Some("vs_a".to_owned()), found.vector_store_id);
    let empty = VectorManagementWriteState::from(read_state(Setup::default()));
    assert!(empty.cache.is_none());
}
