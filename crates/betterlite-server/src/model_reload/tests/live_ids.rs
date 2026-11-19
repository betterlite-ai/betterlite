use crate::model_reload::live_ids::live_model_ids_snapshot;

use super::support::{catalog, deployment, deployment_without_id, ids};

#[test]
fn snapshot_lists_the_published_ids() {
    let published = catalog(&[
        deployment("a", "second", "openai/gpt-4o", true),
        deployment("b", "first", "openai/gpt-4o", true),
    ]);
    assert_eq!(
        live_model_ids_snapshot(Some(&published)),
        ids(&["first", "second"])
    );
}

#[test]
fn snapshot_of_an_absent_router_is_empty() {
    assert!(live_model_ids_snapshot(None).is_empty());
}

#[test]
fn snapshot_skips_a_deployment_without_an_id() {
    let published = catalog(&[
        deployment("a", "kept", "openai/gpt-4o", true),
        deployment_without_id("b"),
    ]);
    assert_eq!(live_model_ids_snapshot(Some(&published)), ids(&["kept"]));
}
