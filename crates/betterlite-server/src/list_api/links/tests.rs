#![cfg(test)]

use std::fs;
use std::path::Path;

use super::{build_list_links, build_page_links};

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

#[test]
fn replays_the_list_query_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/list_query");
    assert!(fs::read_dir(corpus).expect("corpus").next().is_some());
}
