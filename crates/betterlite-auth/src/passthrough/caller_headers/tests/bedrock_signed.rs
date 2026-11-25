#![cfg(test)]

use indexmap::IndexMap;
use secrecy::SecretString;

use super::support::{caller, forwarded_value, headers, secret, settings, sorted_names};
use super::upstream::bedrock_runtime_headers;
use crate::dev_auth_mode::AuthModeFlags;

fn signed(pairs: &[(&str, &str)]) -> IndexMap<String, SecretString> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), secret(value)))
        .collect()
}

#[test]
fn signed_headers_replace_the_callers_and_keep_their_case() {
    let map = headers(&[("x-amz-date", "caller-date"), ("X-Custom", "v")]);
    let signed_headers = signed(&[("X-Amz-Date", "signed-date")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = bedrock_runtime_headers(&map, &caller(None, None), &signed_headers, &config);
    assert_eq!(
        forwarded_value(&forwarded, "X-Amz-Date"),
        Some("signed-date")
    );
    assert!(forwarded_value(&forwarded, "x-amz-date").is_none());
    assert_eq!(sorted_names(&forwarded), vec!["X-Amz-Date", "x-custom"]);
}

#[test]
fn a_signed_header_the_caller_did_not_send_is_added() {
    let map = headers(&[("X-Custom", "v")]);
    let signed_headers = signed(&[("X-Amz-Security-Token", "token")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = bedrock_runtime_headers(&map, &caller(None, None), &signed_headers, &config);
    assert_eq!(
        sorted_names(&forwarded),
        vec!["X-Amz-Security-Token", "x-custom"]
    );
}

#[test]
fn every_litellm_credential_and_hop_header_is_dropped() {
    let map = headers(&[
        ("Authorization", "Bearer x"),
        ("Api-Key", "a"),
        ("X-Api-Key", "b"),
        ("X-Goog-Api-Key", "c"),
        ("Ocp-Apim-Subscription-Key", "d"),
        ("X-Litellm-Api-Key", "e"),
        ("Host", "proxy"),
        ("Content-Length", "10"),
        ("Accept-Encoding", "gzip"),
        ("X-Custom", "v"),
    ]);
    let signed_headers = signed(&[]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = bedrock_runtime_headers(&map, &caller(None, None), &signed_headers, &config);
    assert_eq!(sorted_names(&forwarded), vec!["x-custom"]);
}

#[test]
fn a_header_the_signing_did_not_name_stays() {
    let map = headers(&[("X-Amz-Date", "caller-date")]);
    let signed_headers = signed(&[]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = bedrock_runtime_headers(&map, &caller(None, None), &signed_headers, &config);
    assert_eq!(
        forwarded_value(&forwarded, "x-amz-date"),
        Some("caller-date")
    );
}

#[test]
fn a_lowercased_signed_name_drops_the_callers_header() {
    let map = headers(&[("x-amz-content-sha256", "caller")]);
    let signed_headers = signed(&[("X-Amz-Content-SHA256", "signed")]);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let forwarded = bedrock_runtime_headers(&map, &caller(None, None), &signed_headers, &config);
    assert_eq!(
        forwarded_value(&forwarded, "X-Amz-Content-SHA256"),
        Some("signed")
    );
    assert!(forwarded_value(&forwarded, "x-amz-content-sha256").is_none());
}

#[test]
fn a_header_carrying_the_master_key_is_dropped() {
    let master_key = secret("mk-1");
    let config = settings(Some(&master_key), AuthModeFlags::default(), true, None, &[]);
    let map = headers(&[("X-Caller-Token", "Bearer mk-1"), ("X-Custom", "v")]);
    let signed_headers = signed(&[]);
    let forwarded = bedrock_runtime_headers(&map, &caller(None, None), &signed_headers, &config);
    assert_eq!(sorted_names(&forwarded), vec!["x-custom"]);
}

#[test]
fn a_header_carrying_the_stored_key_is_dropped() {
    let auth = caller(Some("stored-key"), None);
    let config = settings(None, AuthModeFlags::default(), true, None, &[]);
    let map = headers(&[("X-Caller-Token", "stored-key")]);
    let signed_headers = signed(&[]);
    let forwarded = bedrock_runtime_headers(&map, &auth, &signed_headers, &config);
    assert!(sorted_names(&forwarded).is_empty());
}

// signed headers are the proxy's own signature, so they are merged after the caller's secrets are checked
#[test]
fn a_signed_header_is_never_treated_as_a_caller_secret() {
    let master_key = secret("mk-1");
    let config = settings(Some(&master_key), AuthModeFlags::default(), true, None, &[]);
    let signed_headers = signed(&[("Authorization", "mk-1")]);
    let forwarded =
        bedrock_runtime_headers(&headers(&[]), &caller(None, None), &signed_headers, &config);
    assert_eq!(forwarded_value(&forwarded, "Authorization"), Some("mk-1"));
}
