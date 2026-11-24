#![cfg(test)]

use axum::http::Uri;
use betterlite_anthropic::native_compaction::protocol::CompactionProtocol;
use rstest::rstest;

use super::{child_path, child_uri};

#[test]
fn the_child_path_follows_the_protocol() {
    assert_eq!(child_path(CompactionProtocol::Chat), "/v1/chat/completions");
    assert_eq!(child_path(CompactionProtocol::Messages), "/v1/messages");
}

#[rstest]
#[case(
    CompactionProtocol::Chat,
    "/v1/chat/completions?stream=true",
    "",
    "/v1/chat/completions"
)]
#[case(
    CompactionProtocol::Messages,
    "/v1/messages?api-version=2",
    "/",
    "/v1/messages"
)]
#[case(
    CompactionProtocol::Chat,
    "/v1/chat/completions",
    "/tenant-a/",
    "/tenant-a/v1/chat/completions"
)]
#[case(
    CompactionProtocol::Messages,
    "/anthropic/v1/messages",
    "/tenant-a//",
    "/tenant-a/v1/messages"
)]
#[case(
    CompactionProtocol::Chat,
    "https://gateway.internal:4000/base/v1/chat/completions?x=1",
    "/tenant-a/",
    "https://gateway.internal:4000/tenant-a/v1/chat/completions"
)]
#[test]
fn the_child_url_replaces_the_parent_path_and_drops_the_query(
    #[case] protocol: CompactionProtocol,
    #[case] parent: &'static str,
    #[case] root_path: &'static str,
    #[case] expected: &'static str,
) {
    let child = child_uri(&Uri::from_static(parent), root_path, protocol).unwrap();
    assert_eq!(child, Uri::from_static(expected));
    assert_eq!(child.query(), None);
}

#[rstest]
#[case("tenant-a")]
#[case("/tenant a")]
#[test]
fn a_root_path_that_is_not_a_uri_path_is_refused(#[case] root_path: &'static str) {
    child_uri(
        &Uri::from_static("/v1/messages"),
        root_path,
        CompactionProtocol::Messages,
    )
    .expect_err("a root path that is not a URI path cannot build the child URL");
}
