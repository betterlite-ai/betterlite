#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::merge;
use crate::helicone::event::ProxyServerRequest;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn request(headers: &[(&str, &str)]) -> ProxyServerRequest {
    ProxyServerRequest {
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

#[test]
fn header_metadata_takes_only_the_literal_prefix() {
    let headers = request(&[
        ("helicone_property_team", "search"),
        ("helicone-property-team", "hyphen"),
        ("Helicone_User", "capital"),
        ("x-helicone_other", "inner"),
        ("authorization", "Bearer sk"),
    ]);
    let merged = merge(&IndexMap::new(), Some(&headers));
    assert_eq!(
        merged,
        IndexMap::from([("helicone_property_team".to_owned(), text("search"))])
    );
}

#[test]
fn header_metadata_beats_a_metadata_key_of_the_same_name() {
    let metadata = IndexMap::from([
        ("helicone_user".to_owned(), text("from metadata")),
        ("other".to_owned(), text("kept")),
    ]);
    let headers = request(&[("helicone_user", "from header")]);
    let merged = merge(&metadata, Some(&headers));
    assert_eq!(merged["helicone_user"], text("from header"));
    assert_eq!(merged["other"], text("kept"));
}

#[test]
fn header_metadata_does_not_change_the_original() {
    let metadata = IndexMap::from([("a".to_owned(), text("1"))]);
    let before = metadata.clone();
    let headers = request(&[("helicone_b", "2")]);
    let merged = merge(&metadata, Some(&headers));
    assert_eq!(metadata, before);
    assert_eq!(merged.len(), 2);
}

#[test]
fn header_metadata_without_a_proxy_request_returns_the_metadata() {
    let metadata = IndexMap::from([("a".to_owned(), text("1"))]);
    assert_eq!(merge(&metadata, None), metadata);
}

#[test]
fn header_metadata_with_no_headers_returns_the_metadata() {
    let metadata = IndexMap::from([("a".to_owned(), text("1"))]);
    assert_eq!(merge(&metadata, Some(&request(&[]))), metadata);
}
