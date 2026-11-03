#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use super::{build_list_links, build_page_links};
use crate::list_api::plan::query::decode_list_query;
use rstest::rstest;

#[test]
fn links_preserve_ordered_repeated_parameters_and_replace_page() {
    let links = build_list_links(
        "/management/v1/widgets",
        &[
            ("sort".to_owned(), "name".to_owned()),
            ("tag".to_owned(), "a b".to_owned()),
            ("page".to_owned(), "9".to_owned()),
            ("tag".to_owned(), "c".to_owned()),
        ],
        2,
        3,
    );
    assert_eq!(
        links.self_link,
        "/management/v1/widgets?sort=name&tag=a+b&tag=c&page=2"
    );
    assert_eq!(
        links.first,
        "/management/v1/widgets?sort=name&tag=a+b&tag=c&page=1"
    );
    assert_eq!(
        links.last,
        "/management/v1/widgets?sort=name&tag=a+b&tag=c&page=3"
    );
    assert_eq!(
        links.prev,
        Some("/management/v1/widgets?sort=name&tag=a+b&tag=c&page=1".to_owned())
    );
    assert_eq!(
        links.next,
        Some("/management/v1/widgets?sort=name&tag=a+b&tag=c&page=3".to_owned())
    );
}

#[test]
fn pagination_links_cover_empty_and_page_boundaries() {
    let empty = build_list_links("/management/v1/widgets", &[], 1, 0);
    assert_eq!(empty.first, "/management/v1/widgets?page=1");
    assert_eq!(empty.last, "/management/v1/widgets?page=1");

    let page_links = build_page_links("/management/v1/widgets", &[], 2, true);
    assert_eq!(
        page_links.prev,
        Some("/management/v1/widgets?page=1".to_owned())
    );
    assert_eq!(
        page_links.next,
        Some("/management/v1/widgets?page=3".to_owned())
    );
}

#[rstest]
fn replays_the_list_query_corpus(#[files("fuzz/corpus/list_query/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus seed is readable");
    let decoded = decode_list_query(&bytes);
    let expected: BTreeMap<String, String> = decoded.pairs.iter().cloned().collect();
    assert_eq!(decoded.last_value, expected);
}
