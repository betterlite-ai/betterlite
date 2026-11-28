#![cfg(test)]

use super::{BlogPost, ParseFailure, parse_rss_to_posts};
use std::fs;
use std::path::PathBuf;

fn post(title: &str, description: &str, date: &str, url: &str) -> BlogPost {
    BlogPost {
        title: title.to_owned(),
        description: description.to_owned(),
        date: date.to_owned(),
        url: url.to_owned(),
    }
}

fn title_nested_in(inline_elements: usize) -> String {
    let mut xml = String::from("<rss><channel><item><title>Deep");
    for _ in 0..inline_elements {
        xml.push_str("<b>");
    }
    for _ in 0..inline_elements {
        xml.push_str("</b>");
    }
    xml.push_str("</title><link>https://deep</link></item></channel></rss>");
    xml
}

#[test]
fn resolves_entity_and_numeric_references() {
    let xml = "<rss><channel><item><title>A &amp; B</title>\
        <link>https://example.test/q?a=1&amp;b=2</link>\
        <description>zz&#233;zz</description></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
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
fn undefined_entity_is_a_parse_failure() {
    let xml = "<rss><channel><item><title>A &bogus; B</title>\
        <link>https://x</link></item></channel></rss>";
    let error = parse_rss_to_posts(xml, 1, 64).unwrap_err();
    assert!(matches!(error, ParseFailure::Escape(_)));
}

#[test]
fn empty_self_closing_fields_are_present_but_blank() {
    let xml = "<rss><channel><item><title/><link/><description/><pubDate/></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("", "", "", "")]);
}

#[test]
fn only_the_first_direct_child_of_each_name_is_kept() {
    let xml = "<rss><channel><item>\
        <title>First</title><title>Second</title>\
        <link>https://first</link></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("First", "", "", "https://first")]);
}

#[test]
fn nested_elements_do_not_count_as_direct_children() {
    let xml = "<rss><channel><item>\
        <title>Outer<title>Inner</title></title>\
        <link>https://outer</link></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("Outer", "", "", "https://outer")]);
}

#[test]
fn nested_child_tail_matches_element_text() {
    let xml = "<rss><channel><item><title>A<b>B</b>C</title>\
        <link>https://outer</link></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("A", "", "", "https://outer")]);
}

#[test]
fn first_empty_channel_stops_channel_selection() {
    let xml = "<rss><channel/><channel><item><title>Later</title>\
        <link>https://later</link></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, Vec::new());
}

#[test]
fn namespace_qualified_channel_is_not_selected() {
    let error = parse_rss_to_posts("<rss xmlns=\"urn:rss\"><channel/></rss>", 1, 64).unwrap_err();
    assert!(matches!(error, ParseFailure::MissingChannel));
}

#[test]
fn namespaced_field_does_not_shadow_a_plain_one() {
    let xml = "<rss><channel><item><title xmlns=\"urn:t\">Fake</title>\
        <title>Real</title><link>https://real</link></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("Real", "", "", "https://real")]);
}

#[test]
fn malformed_document_boundaries_are_rejected() {
    drop(parse_rss_to_posts("<rss><channel/></rss>tail", 1, 64).unwrap_err());
    drop(parse_rss_to_posts("<rss><channel/></rss><extra/>", 1, 64).unwrap_err());
    drop(parse_rss_to_posts("<rss><channel a='unterminated></rss>", 1, 64).unwrap_err());
}

#[test]
fn validates_the_whole_document_even_past_max_posts() {
    let xml = "<rss><channel>\
        <item><title>One</title><link>https://one</link></item>\
        <item><title>Two</title><link>https://two</link></item>\
        <item><title>Three</title><link>https://three</link>";
    let error = parse_rss_to_posts(xml, 1, 64).unwrap_err();
    assert!(matches!(error, ParseFailure::UnclosedElement));
}

#[test]
fn caps_collected_posts_without_stopping_early() {
    let xml = "<rss><channel>\
        <item><title>One</title><link>https://one</link></item>\
        <item><title>Two</title><link>https://two</link></item>\
        </channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("One", "", "", "https://one")]);
}

#[test]
fn missing_channel_is_reported() {
    let error = parse_rss_to_posts("<rss></rss>", 1, 64).unwrap_err();
    assert!(matches!(error, ParseFailure::MissingChannel));
}

#[test]
fn unclosed_element_at_eof_is_reported() {
    let error = parse_rss_to_posts("<rss><channel><item><title>X", 1, 64).unwrap_err();
    assert!(matches!(error, ParseFailure::UnclosedElement));
}

#[test]
fn mismatched_end_tag_is_a_syntax_error() {
    let error = parse_rss_to_posts("<rss><channel></rss></channel>", 1, 64).unwrap_err();
    assert!(matches!(error, ParseFailure::Xml(_)));
}

#[test]
fn channel_nested_inside_another_element_is_not_the_channel() {
    let xml = "<rss><wrapper><channel><item>\
        <title>X</title><link>https://x</link></item></channel></wrapper></rss>";
    let error = parse_rss_to_posts(xml, 1, 64).unwrap_err();
    assert!(matches!(error, ParseFailure::MissingChannel));
}

#[test]
fn item_nested_inside_another_element_is_not_collected() {
    let xml = "<rss><channel><wrapper><item>\
        <title>X</title><link>https://x</link></item></wrapper></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, Vec::new());
}

#[test]
fn item_element_as_the_root_is_not_collected() {
    let xml = "<item><channel/><title>T</title><link>https://t</link></item>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, Vec::new());
}

#[test]
fn a_nested_channel_does_not_close_the_real_channel() {
    let xml = "<rss><channel><channel/><item><title>T</title>\
        <link>https://t</link></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("T", "", "", "https://t")]);
}

#[test]
fn a_nested_item_does_not_close_the_real_item() {
    let xml = "<rss><channel><item><title>T</title><item/>\
        <link>https://t</link></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("T", "", "", "https://t")]);
}

#[test]
fn item_missing_title_or_link_is_skipped() {
    let xml = "<rss><channel>\
        <item><description>x</description><link>https://x</link></item>\
        <item><title>Valid</title><link>https://valid</link></item>\
        </channel></rss>";
    let posts = parse_rss_to_posts(xml, 2, 64).unwrap();
    assert_eq!(posts, vec![post("Valid", "", "", "https://valid")]);
}

#[test]
fn pub_date_is_formatted_into_the_post() {
    let xml = "<rss><channel><item><title>T</title><link>https://x</link>\
        <pubDate>Thu, 05 Mar 2026 10:00:00 GMT</pubDate></item></channel></rss>";
    let posts = parse_rss_to_posts(xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("T", "", "2026-03-05", "https://x")]);
}

#[test]
fn nesting_at_the_limit_is_accepted() {
    let xml = title_nested_in(60);
    let posts = parse_rss_to_posts(&xml, 1, 64).unwrap();
    assert_eq!(posts, vec![post("Deep", "", "", "https://deep")]);
}

#[test]
fn nesting_past_the_limit_is_refused() {
    let xml = title_nested_in(61);
    let error = parse_rss_to_posts(&xml, 1, 64).unwrap_err();
    assert!(matches!(error, ParseFailure::TooDeep { max_depth: 64 }));
    assert!(error.to_string().contains("64"));
}

#[test]
fn replays_the_blog_feed_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/blog_feed");
    let replayed = fs::read_dir(corpus)
        .expect("the blog feed corpus should exist")
        .map(|entry| {
            let path = entry.expect("a corpus seed should be readable").path();
            let xml = fs::read_to_string(path).expect("a corpus seed should be text");
            drop(parse_rss_to_posts(&xml, 1, 64));
        })
        .count();
    assert!(replayed >= 11, "replayed {replayed} seeds");
}
