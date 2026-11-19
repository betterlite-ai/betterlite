use serde_json::json;

use super::{object, rewriter};
use crate::passthrough::websocket::resolved_vertex_live_setup;

#[test]
fn without_a_rewriter_the_setup_is_returned_unchanged() {
    let setup = object(json!({"model": "alias", "temperature": 1}));
    assert_eq!(resolved_vertex_live_setup(&setup, None), setup);
}

#[test]
fn the_rewritten_model_replaces_the_alias() {
    let setup = object(json!({"model": "alias", "temperature": 1}));
    assert_eq!(
        resolved_vertex_live_setup(&setup, Some(&rewriter)),
        object(json!({
            "model": "projects/p/locations/l/models/alias",
            "temperature": 1
        }))
    );
}

#[test]
fn a_setup_without_a_string_model_is_returned_unchanged() {
    let setup = object(json!({"model": 3, "temperature": 1}));
    assert_eq!(resolved_vertex_live_setup(&setup, Some(&rewriter)), setup);
}

#[test]
fn a_setup_without_a_model_is_returned_unchanged() {
    let setup = object(json!({"temperature": 1}));
    assert_eq!(resolved_vertex_live_setup(&setup, Some(&rewriter)), setup);
}
