#![cfg(test)]

use super::{BlogPost, ParseFailure, parse_rss_to_posts};

fn post(title: &str, description: &str, date: &str, url: &str) -> BlogPost {
    BlogPost {
        title: title.to_owned(),
        description: description.to_owned(),
        date: date.to_owned(),
        url: url.to_owned(),
    }
}

#[test]
fn resolves_entity_and_numeric_references() {
    let xml = "<rss><channel><item><title>A &amp; B</title>\
        <link>https://example.test/q?a=1&amp;b=2</link>\
        <description>zz&#233;zz</description></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1).unwrap();
    assert_eq!(
        posts,
        vec![post(
            "A & B",
            "zz\u{e9}zz",
            "",
            "https://example.test/q?a=1&b=2"
        )]
    );
}

#[test]
fn empty_self_closing_fields_are_present_but_blank() {
    let xml = "<rss><channel><item><title/><link/></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1).unwrap();
    assert_eq!(posts, vec![post("", "", "", "")]);
}

#[test]
fn only_the_first_direct_child_of_each_name_is_kept() {
    let xml = "<rss><channel><item>\
        <title>First</title><title>Second</title>\
        <link>https://first</link></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1).unwrap();
    assert_eq!(posts, vec![post("First", "", "", "https://first")]);
}

#[test]
fn nested_elements_do_not_count_as_direct_children() {
    let xml = "<rss><channel><item>\
        <title>Outer<title>Inner</title></title>\
        <link>https://outer</link></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1).unwrap();
    assert_eq!(posts, vec![post("Outer", "", "", "https://outer")]);
}

#[test]
fn validates_the_whole_document_even_past_max_posts() {
    let xml = "<rss><channel>\
        <item><title>One</title><link>https://one</link></item>\
        <item><title>Two</title><link>https://two</link></item>\
        <item><title>Three</title><link>https://three</link>";
    let error = parse_rss_to_posts(xml, 1).unwrap_err();
    assert!(matches!(error, ParseFailure::UnclosedElement));
}

#[test]
fn caps_collected_posts_without_stopping_early() {
    let xml = "<rss><channel>\
        <item><title>One</title><link>https://one</link></item>\
        <item><title>Two</title><link>https://two</link></item>\
        </channel></rss>";
    let posts = parse_rss_to_posts(xml, 1).unwrap();
    assert_eq!(posts, vec![post("One", "", "", "https://one")]);
}

#[test]
fn missing_channel_is_reported() {
    let error = parse_rss_to_posts("<rss></rss>", 1).unwrap_err();
    assert!(matches!(error, ParseFailure::MissingChannel));
}

#[test]
fn unclosed_element_at_eof_is_reported() {
    let error = parse_rss_to_posts("<rss><channel><item><title>X", 1).unwrap_err();
    assert!(matches!(error, ParseFailure::UnclosedElement));
}

#[test]
fn mismatched_end_tag_is_a_syntax_error() {
    let error = parse_rss_to_posts("<rss><channel></rss></channel>", 1).unwrap_err();
    assert!(matches!(error, ParseFailure::Xml(_)));
}

#[test]
fn channel_nested_inside_another_element_is_not_the_channel() {
    let xml = "<rss><wrapper><channel><item>\
        <title>X</title><link>https://x</link></item></channel></wrapper></rss>";
    let error = parse_rss_to_posts(xml, 1).unwrap_err();
    assert!(matches!(error, ParseFailure::MissingChannel));
}

#[test]
fn item_nested_inside_another_element_is_not_collected() {
    let xml = "<rss><channel><wrapper><item>\
        <title>X</title><link>https://x</link></item></wrapper></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1).unwrap();
    assert_eq!(posts, Vec::new());
}

#[test]
fn item_missing_title_or_link_is_skipped() {
    let xml = "<rss><channel>\
        <item><description>x</description><link>https://x</link></item>\
        <item><title>Valid</title><link>https://valid</link></item>\
        </channel></rss>";
    let posts = parse_rss_to_posts(xml, 2).unwrap();
    assert_eq!(posts, vec![post("Valid", "", "", "https://valid")]);
}
