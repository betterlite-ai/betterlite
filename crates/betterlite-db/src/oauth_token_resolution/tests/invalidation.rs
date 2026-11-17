use crate::oauth_token_resolution::invalidate_user_token_cache;

use super::fakes::{CacheEvent, FakeCaches};
use super::support::{block_on, warnings_during};

fn expected_events() -> Vec<CacheEvent> {
    vec![
        CacheEvent::Discovery {
            server_id: "s1".to_owned(),
        },
        CacheEvent::Store {
            user_id: "u1".to_owned(),
            server_id: "s1".to_owned(),
        },
        CacheEvent::Legacy {
            user_id: "u1".to_owned(),
            server_id: "s1".to_owned(),
        },
    ]
}

#[test]
fn invalidate_drops_the_discovery_store_and_legacy_entries_in_order() {
    let caches = FakeCaches::working();
    block_on(invalidate_user_token_cache(&caches, "u1", "s1"));
    assert_eq!(caches.events(), expected_events());
}

#[test]
fn invalidate_logs_a_store_failure_and_still_drops_the_legacy_entry() {
    let caches = FakeCaches::failing_store();
    let warnings = warnings_during(|| block_on(invalidate_user_token_cache(&caches, "u1", "s1")));
    assert_eq!(caches.events(), expected_events());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains(
        "Failed to invalidate cached MCP OAuth token for user=u1 server=s1: the fake cache failed"
    ));
}

#[test]
fn invalidate_logs_a_legacy_failure() {
    let caches = FakeCaches::failing_legacy();
    let warnings = warnings_during(|| block_on(invalidate_user_token_cache(&caches, "u1", "s1")));
    assert_eq!(caches.events(), expected_events());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains(
        "Failed to drop legacy cached MCP OAuth token for user=u1 server=s1: the fake cache failed"
    ));
}
