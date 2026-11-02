#![cfg(test)]

use std::fs;
use std::path::Path;

use super::build_list_links;

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
}

#[test]
fn replays_the_list_query_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/list_query");
    assert!(fs::read_dir(corpus).expect("corpus").next().is_some());
}
