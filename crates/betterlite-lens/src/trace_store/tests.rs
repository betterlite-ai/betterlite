#![cfg(test)]

use rusqlite::Connection;

use super::*;
use crate::models::execution::TracePart;
use crate::models::finding::{Evidence, Role};

fn part(id: &str, parent: &str, content: &str) -> TracePart {
    TracePart {
        execution_id: String::from("e"),
        span_id: String::from(id),
        parent_span_id: String::from(parent),
        name: String::from("name"),
        kind: String::from("kind"),
        content: String::from(content),
        truncated: false,
    }
}

#[test]
fn stores_and_reads_spans() {
    let store = TraceStore::new(Connection::open_in_memory().unwrap()).unwrap();
    store.add(&[part("b", "", "body")]).unwrap();
    assert_eq!(store.count().unwrap(), 1);
    assert_eq!(store.get("b").unwrap().unwrap().content, "body");
    assert_eq!(store.previous("b").unwrap(), "");
}

#[test]
fn evidence_reads_only_matching_side_of_marker() {
    let store = TraceStore::new(Connection::open_in_memory().unwrap()).unwrap();
    store
        .add(&[part("s", "", "left\n[... content omitted ...]\nright")])
        .unwrap();
    let evidence = Evidence {
        execution_id: String::from("e"),
        span_id: String::from("s"),
        quote: String::from("left"),
        role: Role::default(),
    };
    assert!(store.evidence(&evidence).unwrap().is_some());
}

#[test]
fn formula_root_only() {
    let root = part("r", "", &"x".repeat(2_500));
    let child = part("c", "r", &"x".repeat(2_500));
    assert!(overview_content(&root, 1).chars().count() < 2_500);
    assert!(overview_content(&child, 1).chars().count() < 2_500);
}
