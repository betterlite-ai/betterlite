#![cfg(test)]

use super::{object_key, object_url, put_object};

fn header<'call>(call: &'call super::CallRequest, name: &str) -> Option<&'call str> {
    call.headers
        .iter()
        .find(|(present, _)| present.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[test]
fn call_put_object_signs_the_bucket_host_and_the_encoded_key() {
    let call = put_object(
        "us-west-2",
        "litellm-kb-0123abcd",
        "data/file.txt",
        Some("text/plain"),
        b"hello",
    );
    assert_eq!(call.operation, "PutObject");
    assert_eq!(call.service, "s3");
    assert_eq!(call.signing_region, "us-west-2");
    assert_eq!(call.method, "PUT");
    assert_eq!(
        call.url,
        "https://litellm-kb-0123abcd.s3.us-west-2.amazonaws.com/data/file.txt"
    );
    assert_eq!(header(&call, "Content-Type"), Some("text/plain"));
    assert_eq!(call.body, b"hello".to_vec());
}

#[test]
fn call_put_object_hashes_the_body() {
    let call = put_object("us-west-2", "bucket", "data/file.txt", None, b"hello");
    assert_eq!(
        header(&call, "x-amz-content-sha256"),
        Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
    );
}

#[test]
fn call_put_object_uses_the_octet_stream_default() {
    let call = put_object("us-west-2", "bucket", "data/file.txt", None, b"hello");
    assert_eq!(
        header(&call, "Content-Type"),
        Some("application/octet-stream")
    );
    let empty = put_object("us-west-2", "bucket", "data/file.txt", Some(""), b"hello");
    assert_eq!(
        header(&empty, "Content-Type"),
        Some("application/octet-stream")
    );
}

#[test]
fn call_put_object_keeps_a_dotted_bucket_in_the_path() {
    let call = put_object("us-west-2", "litellm.kb", "data/file.txt", None, b"hello");
    assert_eq!(
        call.url,
        "https://s3.us-west-2.amazonaws.com/litellm.kb/data/file.txt"
    );
}

#[test]
fn call_put_object_encodes_every_key_segment_and_keeps_the_slashes() {
    assert_eq!(object_key("data/my file+.txt"), "data/my%20file%2B.txt");
    assert_eq!(
        object_url("us-west-2", "bucket", "data/a/b.txt"),
        "https://bucket.s3.us-west-2.amazonaws.com/data/a/b.txt"
    );
}
