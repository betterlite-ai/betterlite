#![cfg(test)]

use rstest::rstest;
use rusqlite::Connection;

use super::*;
use crate::analysis::primitives::evidence_valid;
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

#[rstest]
#[case::prefix("e", "s", "left", true)]
#[case::suffix("e", "s", "right", true)]
#[case::substring("e", "s", "igh", true)]
#[case::wrong_execution("other", "s", "left", false)]
#[case::wrong_span("e", "other", "left", false)]
#[case::missing_quote("e", "s", "missing", false)]
#[case::across_marker("e", "s", "left\n[... content omitted ...]\nright", false)]
fn evidence_reads_only_matching_side_of_marker(
    #[case] execution: &str,
    #[case] span: &str,
    #[case] quote: &str,
    #[case] valid: bool,
) {
    let store = TraceStore::new(Connection::open_in_memory().unwrap()).unwrap();
    store
        .add(&[part("s", "", "left\n[... content omitted ...]\nright")])
        .unwrap();
    let evidence = Evidence {
        execution_id: String::from(execution),
        span_id: String::from(span),
        quote: String::from(quote),
        role: Role::default(),
    };
    assert_eq!(store.evidence(&evidence).unwrap().is_some(), valid);
    assert_eq!(
        evidence_valid(
            &evidence,
            &[part("s", "", "left\n[... content omitted ...]\nright")]
        ),
        valid
    );
}

#[test]
fn formula_root_only() {
    let root = part("r", "", &"x".repeat(2_500));
    let child = part("c", "r", &"x".repeat(2_500));
    assert!(overview_content(&root, 1).chars().count() < 2_500);
    assert!(overview_content(&child, 1).chars().count() < 2_500);
}

#[test]
fn replaces_spans_and_keeps_unique_reads() {
    let store = trace_store().unwrap();
    store
        .add(&[part("b", "", "old"), part("a", "", "first")])
        .unwrap();
    store.add(&[part("b", "", "new")]).unwrap();
    store
        .add_reads(&[part("b", "", "read"), part("b", "", "read")])
        .unwrap();
    assert_eq!(store.count().unwrap(), 2);
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM reads", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(store.previous("b").unwrap(), "a");
    assert!(store.get("absent").unwrap().is_none());
    let parts: Vec<_> = store.parts().collect::<Result<_, _>>().unwrap();
    assert_eq!(parts, vec![part("a", "", "first"), part("b", "", "new")]);
    let quote = Evidence {
        execution_id: String::from("e"),
        span_id: String::from("b"),
        quote: String::from("read"),
        role: Role::default(),
    };
    assert_eq!(store.evidence(&quote).unwrap().unwrap().content, "read");
    let directory = store.directory.as_ref().unwrap().path().to_path_buf();
    assert!(directory.join("trace.sqlite").is_file());
    drop(store);
    assert!(!directory.exists());
}

#[test]
fn catalogs_are_bounded_and_sorted() {
    let store = trace_store().unwrap();
    let parts: Vec<_> = (0_u32..30)
        .map(|number| part(&format!("{number:02}"), "", &"\u{e9}".repeat(2500)))
        .collect();
    store.add(&parts).unwrap();
    let pages: Vec<_> = store.catalogs(1).collect::<Result<_, _>>().unwrap();
    assert!(pages.len() > 1);
    assert!(
        pages
            .iter()
            .all(|page| serde_json::to_vec(page).unwrap().len() <= 24_000)
    );
    let rows: Vec<_> = pages.into_iter().flatten().collect();
    assert_eq!(rows.len(), 30);
    assert_eq!(
        rows.first().unwrap(),
        &(
            String::from("00"),
            String::new(),
            String::from("name"),
            String::from("kind"),
            overview_content(parts.first().unwrap(), 1)
        )
    );
    store.add(&[part("huge", &"p".repeat(24_000), "")]).unwrap();
    assert!(store.catalogs(1).any(|page| page.is_err()));
}

#[test]
fn catalog_boundary_includes_json_framing() {
    let store = trace_store().unwrap();
    let mut span = part("s", "", "");
    store.add(&[span.clone()]).unwrap();
    let baseline = serde_json::to_vec(&store.catalogs(1).next().unwrap().unwrap())
        .unwrap()
        .len();
    span.name.push_str(&"x".repeat(24_000 - baseline));
    store.add(&[span.clone()]).unwrap();
    let page = store.catalogs(1).next().unwrap().unwrap();
    assert_eq!(serde_json::to_vec(&page).unwrap().len(), 24_000);
    span.name.push('x');
    store.add(&[span]).unwrap();
    assert!(matches!(
        store.catalogs(1).next().unwrap(),
        Err(TraceStoreError::Oversized)
    ));
}
